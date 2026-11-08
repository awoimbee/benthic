//! A minimal Git smart-HTTP client, sufficient for Subsurface's cloud
//! repositories.
//!
//! Subsurface cloud storage is a normal bare git repository per account:
//! HTTPS Basic auth, the user's email as both repository name and branch, and
//! a tree of small text blobs. We only need the shallow tip of one branch to
//! read it, and one new commit to write it, so this module implements just
//! that: ref discovery, `upload-pack` with `deepen 1`, pack parsing, and a
//! `receive-pack` push without deltas.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read, Write};

use base64::Engine;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha1::{Digest, Sha1};

use super::http::{self, Method};

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// The tip of a branch and every file in its tree.
pub struct Fetched {
    pub commit: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone)]
struct GitObject {
    kind: &'static str,
    data: Vec<u8>,
}

struct TreeEntry {
    mode: u32,
    name: String,
    oid: String,
}

fn auth_headers(email: &str, password: &str) -> Vec<(&'static str, String)> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{password}"));
    vec![
        ("Authorization", format!("Basic {encoded}")),
        ("Accept", "*/*".to_string()),
    ]
}

fn header_refs<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    headers.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// Fetch the shallow tip of `branch` from the repository at `base`.
///
/// Returns `Ok(None)` when the branch does not exist yet (a new account).
pub async fn fetch_branch(
    base: &str,
    branch: &str,
    email: &str,
    password: &str,
) -> Result<Option<Fetched>, String> {
    let base = base.trim_end_matches('/');
    let headers = auth_headers(email, password);
    let (refs, caps) = discover(base, "upload-pack", &headers).await?;
    let ref_name = format!("refs/heads/{branch}");
    let Some(commit) = refs.get(&ref_name).cloned() else {
        return Ok(None);
    };

    let mut request = Vec::new();
    let mut want = format!("want {commit}");
    for cap in [
        "multi_ack_detailed",
        "side-band-64k",
        "ofs-delta",
        "no-progress",
    ] {
        if caps.iter().any(|c| c == cap) {
            want.push(' ');
            want.push_str(cap);
        }
    }
    want.push('\n');
    pkt(&mut request, want.as_bytes());
    pkt(&mut request, b"deepen 1\n");
    request.extend_from_slice(b"0000");
    pkt(&mut request, b"done\n");

    let url = format!("{base}/git-upload-pack");
    let mut all_headers = header_refs(&headers);
    all_headers.push(("Content-Type", "application/x-git-upload-pack-request"));
    all_headers.push(("Accept", "application/x-git-upload-pack-result"));
    let response =
        http::request_raw(Method::Post, &url, &all_headers, Some(&request), None).await?;
    if !response.ok() {
        let text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Subsurface cloud returned {}: {}",
            response.status,
            text.chars().take(300).collect::<String>()
        ));
    }
    let pack = sideband_pack(&response.body)?;
    let objects = parse_pack(&pack)?;
    let Some(commit_object) = objects.get(&commit) else {
        return Err("Subsurface cloud response did not include the branch tip".to_string());
    };
    let tree = commit_tree(&commit_object.data)?;
    let mut files = BTreeMap::new();
    collect_tree(&objects, &tree, "", &mut files)?;
    Ok(Some(Fetched { commit, files }))
}

/// Push `files` to `branch`, creating a new commit on top of `expected` (or
/// the current tip when the branch is new). `expected` is the revision the
/// caller last saw; a mismatch aborts before anything is written.
pub async fn push_branch(
    base: &str,
    branch: &str,
    email: &str,
    password: &str,
    expected: Option<&str>,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
    let base = base.trim_end_matches('/');
    let headers = auth_headers(email, password);
    let (refs, caps) = discover(base, "receive-pack", &headers).await?;
    let ref_name = format!("refs/heads/{branch}");
    let old = refs
        .get(&ref_name)
        .cloned()
        .unwrap_or_else(|| ZERO_OID.to_string());
    if let Some(expected) = expected {
        if expected != old {
            return Err(
                "The remote branch changed since the last sync. Fetch again first.".to_string(),
            );
        }
    }

    let tree_oid = build_tree(&files)?;
    let now = crate::platform::now_secs();
    let mut commit_text = format!("tree {tree_oid}\n");
    if old != ZERO_OID {
        commit_text.push_str(&format!("parent {old}\n"));
    }
    commit_text.push_str(&format!(
        "author benthic <benthic@localhost> {now} +0000\ncommitter benthic <benthic@localhost> {now} +0000\n\nbenthic: sync dive log\n"
    ));
    let commit_oid = hash_object("commit", commit_text.as_bytes());
    let objects = build_objects(&files)?;
    let pack = build_pack(&objects, &tree_oid, &commit_oid, commit_text.as_bytes())?;

    let mut request = Vec::new();
    let mut caps_used = String::new();
    if caps.iter().any(|c| c == "report-status") {
        caps_used.push_str("report-status");
    }
    let update = format!("{old} {commit_oid} {ref_name}\0{caps_used}\n");
    pkt(&mut request, update.as_bytes());
    request.extend_from_slice(b"0000");
    request.extend_from_slice(&pack);

    let url = format!("{base}/git-receive-pack");
    let mut all_headers = header_refs(&headers);
    all_headers.push(("Content-Type", "application/x-git-receive-pack-request"));
    all_headers.push(("Accept", "application/x-git-receive-pack-result"));
    let response =
        http::request_raw(Method::Post, &url, &all_headers, Some(&request), None).await?;
    if !response.ok() {
        let text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Subsurface cloud returned {}: {}",
            response.status,
            text.chars().take(300).collect::<String>()
        ));
    }
    let status = String::from_utf8_lossy(&response.body);
    if status.contains("unpack error") || status.contains("ng ") {
        return Err(format!(
            "Subsurface cloud rejected the push: {}",
            status.trim()
        ));
    }
    Ok(commit_oid)
}

async fn discover(
    base: &str,
    service: &str,
    headers: &[(&'static str, String)],
) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let url = format!("{base}/info/refs?service=git-{service}");
    let response = http::request_raw(Method::Get, &url, &header_refs(headers), None, None).await?;
    if !response.ok() {
        let text = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Subsurface cloud rejected the credentials ({}): {}",
            response.status,
            text.chars().take(200).collect::<String>()
        ));
    }
    let mut refs = BTreeMap::new();
    let mut caps = Vec::new();
    let mut position = 0usize;
    while position < response.body.len() {
        let Some((payload, consumed)) = read_pkt(&response.body[position..]) else {
            break;
        };
        position += consumed;
        if payload.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(&payload);
        let text = text.trim_end_matches('\n');
        if text.starts_with("# service=") {
            continue;
        }
        let Some((oid, rest)) = text.split_once(' ') else {
            continue;
        };
        let (name, capabilities) = match rest.split_once('\0') {
            Some((name, caps)) => (name, Some(caps)),
            None => (rest, None),
        };
        if let Some(capabilities) = capabilities {
            caps = capabilities.split(' ').map(str::to_string).collect();
        }
        refs.insert(name.to_string(), oid.to_string());
        if let Some(head) = refs.get("HEAD").cloned() {
            if let Some(target) = caps.iter().find_map(|c| c.strip_prefix("symref=HEAD:")) {
                refs.insert(target.to_string(), head);
            }
        }
    }
    Ok((refs, caps))
}

fn pkt(out: &mut Vec<u8>, payload: &[u8]) {
    let length = payload.len() + 4;
    out.extend_from_slice(format!("{length:04x}").as_bytes());
    out.extend_from_slice(payload);
}

fn read_pkt(data: &[u8]) -> Option<(Vec<u8>, usize)> {
    if data.len() < 4 {
        return None;
    }
    let length = usize::from_str_radix(std::str::from_utf8(&data[..4]).ok()?, 16).ok()?;
    if length == 0 {
        return Some((Vec::new(), 4));
    }
    if length < 4 || data.len() < length {
        return None;
    }
    Some((data[4..length].to_vec(), length))
}

fn sideband_pack(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut pack = Vec::new();
    let mut position = 0usize;
    while position < body.len() {
        let Some((payload, consumed)) = read_pkt(&body[position..]) else {
            break;
        };
        position += consumed;
        if payload.is_empty() {
            continue;
        }
        match payload[0] {
            1 => pack.extend_from_slice(&payload[1..]),
            2 => {}
            3 => {
                return Err(String::from_utf8_lossy(&payload[1..]).trim().to_string());
            }
            _ => {
                let text = String::from_utf8_lossy(&payload);
                if text.starts_with("PACK") {
                    pack.extend_from_slice(&payload);
                }
            }
        }
    }
    if pack.is_empty() {
        return Err("Subsurface cloud returned no git pack".to_string());
    }
    Ok(pack)
}

fn parse_pack(pack: &[u8]) -> Result<HashMap<String, GitObject>, String> {
    if pack.len() < 12 || &pack[..4] != b"PACK" {
        return Err("invalid git pack".to_string());
    }
    let count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]) as usize;
    let mut objects: HashMap<String, GitObject> = HashMap::new();
    let mut by_offset: HashMap<usize, (String, GitObject)> = HashMap::new();
    let mut pending: Vec<(String, usize, Vec<u8>)> = Vec::new();
    let mut position = 12usize;
    for _ in 0..count {
        let object_start = position;
        let mut byte = pack[position];
        position += 1;
        let kind = (byte >> 4) & 7;
        let mut size = (byte & 0x0f) as usize;
        let mut shift = 4;
        while byte & 0x80 != 0 {
            byte = pack[position];
            position += 1;
            size |= ((byte & 0x7f) as usize) << shift;
            shift += 7;
        }
        match kind {
            1..=4 => {
                let (data, consumed) = inflate(&pack[position..], size)?;
                position += consumed;
                let kind = match kind {
                    1 => "commit",
                    2 => "tree",
                    3 => "blob",
                    _ => "tag",
                };
                let oid = hash_object(kind, &data);
                let object = GitObject { kind, data };
                objects.insert(oid.clone(), object.clone());
                by_offset.insert(object_start, (oid, object));
            }
            6 => {
                let mut byte = pack[position];
                position += 1;
                let mut offset = (byte & 0x7f) as usize;
                while byte & 0x80 != 0 {
                    byte = pack[position];
                    position += 1;
                    offset = ((offset + 1) << 7) | (byte & 0x7f) as usize;
                }
                let (delta, consumed) = inflate(&pack[position..], size)?;
                position += consumed;
                let base_offset = object_start.checked_sub(offset).ok_or("bad delta offset")?;
                let Some((_, base)) = by_offset.get(&base_offset).cloned() else {
                    return Err("missing delta base".to_string());
                };
                let data = apply_delta(&base.data, &delta)?;
                let oid = hash_object(base.kind, &data);
                let object = GitObject {
                    kind: base.kind,
                    data,
                };
                objects.insert(oid.clone(), object.clone());
                by_offset.insert(object_start, (oid, object));
            }
            7 => {
                let base_oid = hex(&pack[position..position + 20]);
                position += 20;
                let (delta, consumed) = inflate(&pack[position..], size)?;
                position += consumed;
                pending.push((base_oid, object_start, delta));
            }
            _ => return Err(format!("unsupported git object type {kind}")),
        }
    }
    // Resolve ref-deltas once every base is available.
    loop {
        if pending.is_empty() {
            break;
        }
        let mut progress = false;
        pending.retain(|(base_oid, offset, delta)| {
            let Some(base) = objects.get(base_oid).cloned() else {
                return true;
            };
            if let Ok(data) = apply_delta(&base.data, delta) {
                let oid = hash_object(base.kind, &data);
                let object = GitObject {
                    kind: base.kind,
                    data,
                };
                objects.insert(oid.clone(), object.clone());
                by_offset.insert(*offset, (oid, object));
                progress = true;
                false
            } else {
                true
            }
        });
        if !progress {
            return Err("could not resolve git deltas".to_string());
        }
    }
    Ok(objects)
}

fn inflate(data: &[u8], expected: usize) -> Result<(Vec<u8>, usize), String> {
    let mut decoder = ZlibDecoder::new(Cursor::new(data));
    let mut out = vec![0u8; expected];
    decoder
        .read_exact(&mut out)
        .map_err(|error| format!("bad git zlib stream: {error}"))?;
    let consumed = decoder.total_in() as usize;
    Ok((out, consumed))
}

fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, String> {
    let mut position = 0usize;
    let base_size = read_varint(delta, &mut position)?;
    if base_size != base.len() {
        return Err("git delta base size mismatch".to_string());
    }
    let result_size = read_varint(delta, &mut position)?;
    let mut out = Vec::with_capacity(result_size);
    while position < delta.len() {
        let command = delta[position];
        position += 1;
        if command & 0x80 != 0 {
            let mut offset = 0usize;
            let mut size = 0usize;
            for index in 0..4 {
                if command & (1 << index) != 0 {
                    offset |= (delta[position] as usize) << (8 * index);
                    position += 1;
                }
            }
            for index in 0..3 {
                if command & (1 << (4 + index)) != 0 {
                    size |= (delta[position] as usize) << (8 * index);
                    position += 1;
                }
            }
            if size == 0 {
                size = 0x10000;
            }
            if offset + size > base.len() {
                return Err("git delta copy out of range".to_string());
            }
            out.extend_from_slice(&base[offset..offset + size]);
        } else if command != 0 {
            let end = position + command as usize;
            if end > delta.len() {
                return Err("git delta insert out of range".to_string());
            }
            out.extend_from_slice(&delta[position..end]);
            position = end;
        } else {
            return Err("invalid git delta command".to_string());
        }
    }
    if out.len() != result_size {
        return Err("git delta result size mismatch".to_string());
    }
    Ok(out)
}

fn read_varint(data: &[u8], position: &mut usize) -> Result<usize, String> {
    let mut value = 0usize;
    let mut shift = 0;
    loop {
        let byte = *data.get(*position).ok_or("truncated git varint")?;
        *position += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 63 {
            return Err("git varint too long".to_string());
        }
    }
}

fn commit_tree(data: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(data);
    for line in text.lines() {
        if let Some(tree) = line.strip_prefix("tree ") {
            return Ok(tree.trim().to_string());
        }
    }
    Err("git commit has no tree".to_string())
}

fn parse_tree(data: &[u8]) -> Result<Vec<TreeEntry>, String> {
    let mut entries = Vec::new();
    let mut position = 0usize;
    while position < data.len() {
        let space = data[position..]
            .iter()
            .position(|b| *b == b' ')
            .ok_or("bad git tree")?
            + position;
        let mode = u32::from_str_radix(
            std::str::from_utf8(&data[position..space]).map_err(|_| "bad git tree mode")?,
            8,
        )
        .map_err(|_| "bad git tree mode")?;
        let nul = data[space + 1..]
            .iter()
            .position(|b| *b == 0)
            .ok_or("bad git tree name")?
            + space
            + 1;
        let name = String::from_utf8_lossy(&data[space + 1..nul]).into_owned();
        let oid_start = nul + 1;
        if oid_start + 20 > data.len() {
            return Err("truncated git tree".to_string());
        }
        let oid = hex(&data[oid_start..oid_start + 20]);
        entries.push(TreeEntry { mode, name, oid });
        position = oid_start + 20;
    }
    Ok(entries)
}

fn collect_tree(
    objects: &HashMap<String, GitObject>,
    oid: &str,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let tree = objects.get(oid).ok_or("missing git tree object")?;
    for entry in parse_tree(&tree.data)? {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{prefix}/{}", entry.name)
        };
        if entry.mode == 0o40000 {
            collect_tree(objects, &entry.oid, &path, files)?;
        } else if entry.mode & 0o170000 == 0o100000 {
            let Some(blob) = objects.get(&entry.oid) else {
                return Err("missing git blob object".to_string());
            };
            files.insert(path, blob.data.clone());
        }
    }
    Ok(())
}

fn hash_object(kind: &str, data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{kind} {}\0", data.len()).as_bytes());
    hasher.update(data);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn raw_oid(hex: &str) -> Result<[u8; 20], String> {
    if hex.len() != 40 {
        return Err("bad git object id".to_string());
    }
    let mut out = [0u8; 20];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| "bad git object id".to_string())?;
    }
    Ok(out)
}

#[derive(Default)]
struct Node {
    files: BTreeMap<String, (u32, [u8; 20])>,
    dirs: BTreeMap<String, Node>,
}

fn build_tree(files: &BTreeMap<String, Vec<u8>>) -> Result<String, String> {
    let mut root = Node::default();
    for (path, data) in files {
        let mut node = &mut root;
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            node = node.dirs.entry((*part).to_string()).or_default();
        }
        let oid = raw_oid(&hash_object("blob", data))?;
        node.files
            .insert(parts[parts.len() - 1].to_string(), (0o100644, oid));
    }
    write_node(&root)
}

fn write_node(node: &Node) -> Result<String, String> {
    let mut entries: Vec<(String, u32, [u8; 20], bool)> = Vec::new();
    for (name, (mode, oid)) in &node.files {
        entries.push((name.clone(), *mode, *oid, false));
    }
    for (name, child) in &node.dirs {
        let oid = raw_oid(&write_node(child)?)?;
        entries.push((name.clone(), 0o40000, oid, true));
    }
    entries.sort_by(|a, b| {
        let key = |entry: &(String, u32, [u8; 20], bool)| {
            if entry.3 {
                format!("{}/", entry.0)
            } else {
                entry.0.clone()
            }
        };
        key(a).cmp(&key(b))
    });
    let mut data = Vec::new();
    for (name, mode, oid, _) in entries {
        data.extend_from_slice(format!("{mode:o} {name}\0").as_bytes());
        data.extend_from_slice(&oid);
    }
    Ok(hash_object("tree", &data))
}

fn build_objects(files: &BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, GitObject>, String> {
    let mut objects = BTreeMap::new();
    for data in files.values() {
        let oid = hash_object("blob", data);
        objects.insert(
            oid,
            GitObject {
                kind: "blob",
                data: data.clone(),
            },
        );
    }
    let mut root = Node::default();
    for (path, data) in files {
        let mut node = &mut root;
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            node = node.dirs.entry((*part).to_string()).or_default();
        }
        node.files.insert(
            parts[parts.len() - 1].to_string(),
            (0o100644, raw_oid(&hash_object("blob", data))?),
        );
    }
    write_objects(&root, &mut objects)?;
    Ok(objects)
}

fn write_objects(node: &Node, objects: &mut BTreeMap<String, GitObject>) -> Result<String, String> {
    let mut entries: Vec<(String, u32, [u8; 20], bool)> = Vec::new();
    for (name, (mode, oid)) in &node.files {
        entries.push((name.clone(), *mode, *oid, false));
    }
    for (name, child) in &node.dirs {
        let oid = raw_oid(&write_objects(child, objects)?)?;
        entries.push((name.clone(), 0o40000, oid, true));
    }
    entries.sort_by(|a, b| {
        let key = |entry: &(String, u32, [u8; 20], bool)| {
            if entry.3 {
                format!("{}/", entry.0)
            } else {
                entry.0.clone()
            }
        };
        key(a).cmp(&key(b))
    });
    let mut data = Vec::new();
    for (name, mode, oid, _) in entries {
        data.extend_from_slice(format!("{mode:o} {name}\0").as_bytes());
        data.extend_from_slice(&oid);
    }
    let oid = hash_object("tree", &data);
    objects.insert(oid.clone(), GitObject { kind: "tree", data });
    Ok(oid)
}

fn build_pack(
    objects: &BTreeMap<String, GitObject>,
    tree_oid: &str,
    commit_oid: &str,
    commit_data: &[u8],
) -> Result<Vec<u8>, String> {
    let mut all: BTreeMap<String, GitObject> = objects.clone();
    all.insert(
        commit_oid.to_string(),
        GitObject {
            kind: "commit",
            data: commit_data.to_vec(),
        },
    );
    if !all.contains_key(tree_oid) {
        return Err("tree object missing from pack".to_string());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"PACK");
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&(all.len() as u32).to_be_bytes());
    for object in all.values() {
        let kind = match object.kind {
            "commit" => 1u8,
            "tree" => 2,
            "blob" => 3,
            _ => 4,
        };
        let mut size = object.data.len();
        let mut byte = (kind << 4) | (size & 0x0f) as u8;
        size >>= 4;
        if size > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        while size > 0 {
            let mut next = (size & 0x7f) as u8;
            size >>= 7;
            if size > 0 {
                next |= 0x80;
            }
            out.push(next);
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&object.data).map_err(|e| e.to_string())?;
        out.extend_from_slice(&encoder.finish().map_err(|e| e.to_string())?);
    }
    let mut hasher = Sha1::new();
    hasher.update(&out);
    out.extend_from_slice(&hasher.finalize());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip_hashes_and_reads_objects() {
        let mut files = BTreeMap::new();
        files.insert("hello.txt".to_string(), b"hello".to_vec());
        files.insert("dir/world.txt".to_string(), b"world".to_vec());
        let tree = build_tree(&files).unwrap();
        let commit_data = format!("tree {tree}\n\nmessage\n");
        let commit = hash_object("commit", commit_data.as_bytes());
        let objects = build_objects(&files).unwrap();
        let pack = build_pack(&objects, &tree, &commit, commit_data.as_bytes()).unwrap();
        let parsed = parse_pack(&pack).unwrap();
        let parsed_tree = commit_tree(&parsed[&commit].data).unwrap();
        assert_eq!(parsed_tree, tree);
        let mut read = BTreeMap::new();
        collect_tree(&parsed, &tree, "", &mut read).unwrap();
        assert_eq!(read, files);
    }

    #[test]
    fn pkt_lines_roundtrip() {
        let mut bytes = Vec::new();
        pkt(&mut bytes, b"hello\n");
        let (payload, consumed) = read_pkt(&bytes).unwrap();
        assert_eq!(payload, b"hello\n");
        assert_eq!(consumed, bytes.len());
    }
}
