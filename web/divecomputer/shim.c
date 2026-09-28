/*
 * benthic — libdivecomputer wasm shim.
 *
 * Exposes a small C API to JavaScript:
 *   - benthic_dc_descriptors(): the descriptor table as JSON
 *   - benthic_dc_parse(...):     a raw dump as neutral RawDive JSON
 *   - benthic_dc_download(...):  download a device as { error, fingerprint,
 *                                dives: [RawDive, ...] } JSON
 *
 * Device I/O goes through dc_custom_open. Each iostream callback forwards to a
 * JS function on globalThis.benthicHost and wraps the async work in
 * Asyncify.handleAsync, so the synchronous C API can drive async
 * WebSerial/WebBluetooth.
 *
 * The Rust web app deserialises the neutral RawDive JSON and builds the domain
 * model with the shared mapping in benthic-core.
 */

#include <emscripten.h>

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libdivecomputer/context.h>
#include <libdivecomputer/custom.h>
#include <libdivecomputer/descriptor.h>
#include <libdivecomputer/device.h>
#include <libdivecomputer/iterator.h>
#include <libdivecomputer/parser.h>

/* ------------------------------------------------------------------ buffer */

typedef struct {
	char *data;
	size_t len;
	size_t cap;
} buf_t;

static void buf_need(buf_t *b, size_t extra)
{
	if (b->len + extra + 1 <= b->cap)
		return;
	size_t cap = b->cap ? b->cap : 256;
	while (cap < b->len + extra + 1)
		cap *= 2;
	b->data = realloc(b->data, cap);
	b->cap = cap;
}

static void buf_putn(buf_t *b, const char *s, size_t n)
{
	buf_need(b, n);
	memcpy(b->data + b->len, s, n);
	b->len += n;
	b->data[b->len] = '\0';
}

static void buf_put(buf_t *b, const char *s)
{
	buf_putn(b, s, strlen(s));
}

static void buf_printf(buf_t *b, const char *fmt, ...)
{
	char stack[160];
	va_list args;
	va_start(args, fmt);
	int n = vsnprintf(stack, sizeof(stack), fmt, args);
	va_end(args);
	if (n < 0)
		return;
	if ((size_t)n < sizeof(stack)) {
		buf_putn(b, stack, n);
		return;
	}
	buf_need(b, n);
	va_start(args, fmt);
	vsnprintf(b->data + b->len, n + 1, fmt, args);
	va_end(args);
	b->len += n;
}

static void buf_string(buf_t *b, const char *s)
{
	buf_put(b, "\"");
	for (const unsigned char *p = (const unsigned char *)s ? (const unsigned char *)s : (const unsigned char *)""; *p; p++) {
		switch (*p) {
		case '"':  buf_put(b, "\\\""); break;
		case '\\': buf_put(b, "\\\\"); break;
		case '\n': buf_put(b, "\\n"); break;
		case '\r': buf_put(b, "\\r"); break;
		case '\t': buf_put(b, "\\t"); break;
		default:
			if (*p < 0x20)
				buf_printf(b, "\\u%04x", *p);
			else
				buf_putn(b, (const char *)p, 1);
		}
	}
	buf_put(b, "\"");
}

static void hex_encode(buf_t *b, const unsigned char *data, unsigned int size)
{
	for (unsigned int i = 0; i < size; i++)
		buf_printf(b, "%02x", data[i]);
}

static unsigned int hex_decode(const char *hex, unsigned char *out, unsigned int max)
{
	unsigned int n = 0;
	if (!hex)
		return 0;
	for (const char *p = hex; p[0] && p[1] && n < max; p += 2) {
		int hi = (p[0] >= '0' && p[0] <= '9') ? p[0] - '0'
			: (p[0] >= 'a' && p[0] <= 'f') ? p[0] - 'a' + 10
			: (p[0] >= 'A' && p[0] <= 'F') ? p[0] - 'A' + 10 : -1;
		int lo = (p[1] >= '0' && p[1] <= '9') ? p[1] - '0'
			: (p[1] >= 'a' && p[1] <= 'f') ? p[1] - 'a' + 10
			: (p[1] >= 'A' && p[1] <= 'F') ? p[1] - 'A' + 10 : -1;
		if (hi < 0 || lo < 0)
			break;
		out[n++] = (unsigned char)((hi << 4) | lo);
	}
	return n;
}

/* --------------------------------------------------------- enum -> strings */

static void put_usage(buf_t *b, dc_usage_t usage)
{
	switch (usage) {
	case DC_USAGE_OXYGEN:       buf_put(b, "\"oxygen\""); break;
	case DC_USAGE_DILUENT:      buf_put(b, "\"diluent\""); break;
	case DC_USAGE_OPEN_CIRCUIT: buf_put(b, "\"open_circuit\""); break;
	default:                    buf_put(b, "\"none\""); break;
	}
}

static void put_divemode(buf_t *b, dc_divemode_t mode)
{
	switch (mode) {
	case DC_DIVEMODE_FREEDIVE: buf_put(b, "\"freedive\""); break;
	case DC_DIVEMODE_GAUGE:    buf_put(b, "\"gauge\""); break;
	case DC_DIVEMODE_CCR:      buf_put(b, "\"ccr\""); break;
	case DC_DIVEMODE_SCR:      buf_put(b, "\"scr\""); break;
	default:                   buf_put(b, "\"oc\""); break;
	}
}

static void put_deco_kind(buf_t *b, dc_deco_type_t kind)
{
	switch (kind) {
	case DC_DECO_NDL:        buf_put(b, "\"ndl\""); break;
	case DC_DECO_SAFETYSTOP: buf_put(b, "\"safety_stop\""); break;
	case DC_DECO_DEEPSTOP:   buf_put(b, "\"deep_stop\""); break;
	default:                 buf_put(b, "\"deco_stop\""); break;
	}
}

/* ------------------------------------------------------------- descriptors */

EMSCRIPTEN_KEEPALIVE
char *benthic_dc_descriptors(void)
{
	dc_context_t *context = NULL;
	dc_iterator_t *iterator = NULL;
	dc_descriptor_t *descriptor = NULL;
	buf_t b = {0};

	if (dc_context_new(&context) != DC_STATUS_SUCCESS)
		return NULL;
	if (dc_descriptor_iterator_new(&iterator, context) != DC_STATUS_SUCCESS) {
		dc_context_free(context);
		return NULL;
	}

	buf_put(&b, "[");
	int first = 1;
	while (dc_iterator_next(iterator, &descriptor) == DC_STATUS_SUCCESS) {
		if (!first)
			buf_put(&b, ",");
		first = 0;
		buf_put(&b, "{\"vendor\":");
		buf_string(&b, dc_descriptor_get_vendor(descriptor));
		buf_put(&b, ",\"product\":");
		buf_string(&b, dc_descriptor_get_product(descriptor));
		buf_printf(&b, ",\"transports\":%u", dc_descriptor_get_transports(descriptor));
		buf_put(&b, "}");
		dc_descriptor_free(descriptor);
	}
	buf_put(&b, "]");
	dc_iterator_free(iterator);
	dc_context_free(context);

	if (!b.data)
		b.data = strdup("[]");
	return b.data;
}

/* -------------------------------------------------------------- raw sample */

typedef struct {
	buf_t *out;
	int first;
} sample_writer_t;

static void write_sample(dc_sample_type_t type, const dc_sample_value_t *value, void *userdata)
{
	sample_writer_t *w = userdata;

	if (type == DC_SAMPLE_GASMIX && value->gasmix == DC_GASMIX_UNKNOWN)
		return;
	switch (type) {
	case DC_SAMPLE_TIME:
	case DC_SAMPLE_DEPTH:
	case DC_SAMPLE_PRESSURE:
	case DC_SAMPLE_TEMPERATURE:
	case DC_SAMPLE_EVENT:
	case DC_SAMPLE_RBT:
	case DC_SAMPLE_HEARTBEAT:
	case DC_SAMPLE_BEARING:
	case DC_SAMPLE_SETPOINT:
	case DC_SAMPLE_PPO2:
	case DC_SAMPLE_CNS:
	case DC_SAMPLE_DECO:
	case DC_SAMPLE_GASMIX:
	case DC_SAMPLE_TTS:
		break;
	default:
		return;
	}

	if (!w->first)
		buf_put(w->out, ",");
	w->first = 0;

	switch (type) {
	case DC_SAMPLE_TIME:
		buf_printf(w->out, "{\"t\":\"time\",\"ms\":%u}", value->time);
		break;
	case DC_SAMPLE_DEPTH:
		buf_printf(w->out, "{\"t\":\"depth\",\"m\":%.17g}", value->depth);
		break;
	case DC_SAMPLE_PRESSURE:
		buf_printf(w->out, "{\"t\":\"pressure\",\"tank\":%u,\"bar\":%.17g}",
			value->pressure.tank, value->pressure.value);
		break;
	case DC_SAMPLE_TEMPERATURE:
		buf_printf(w->out, "{\"t\":\"temperature\",\"c\":%.17g}", value->temperature);
		break;
	case DC_SAMPLE_EVENT:
		buf_printf(w->out, "{\"t\":\"event\",\"kind\":%u,\"ms\":%u,\"flags\":%u,\"value\":%d",
			value->event.type, value->event.time, value->event.flags, (int)value->event.value);
		if (value->event.name) {
			buf_put(w->out, ",\"name\":");
			buf_string(w->out, value->event.name);
		}
		buf_put(w->out, "}");
		break;
	case DC_SAMPLE_RBT:
		buf_printf(w->out, "{\"t\":\"rbt\",\"s\":%u}", value->rbt);
		break;
	case DC_SAMPLE_HEARTBEAT:
		buf_printf(w->out, "{\"t\":\"heartbeat\",\"bpm\":%u}", value->heartbeat);
		break;
	case DC_SAMPLE_BEARING:
		buf_printf(w->out, "{\"t\":\"bearing\",\"deg\":%u}", value->bearing);
		break;
	case DC_SAMPLE_SETPOINT:
		buf_printf(w->out, "{\"t\":\"setpoint\",\"bar\":%.17g}", value->setpoint);
		break;
	case DC_SAMPLE_PPO2:
		buf_printf(w->out, "{\"t\":\"ppo2\",\"sensor\":%u,\"bar\":%.17g}",
			value->ppo2.sensor, value->ppo2.value);
		break;
	case DC_SAMPLE_CNS:
		buf_printf(w->out, "{\"t\":\"cns\",\"fraction\":%.17g}", value->cns);
		break;
	case DC_SAMPLE_DECO:
		buf_put(w->out, "{\"t\":\"deco\",\"kind\":");
		put_deco_kind(w->out, value->deco.type);
		buf_printf(w->out, ",\"s\":%u,\"m\":%.17g,\"tts\":%u}",
			value->deco.time, value->deco.depth, value->deco.tts);
		break;
	case DC_SAMPLE_GASMIX:
		buf_printf(w->out, "{\"t\":\"gasmix\",\"index\":%u}", value->gasmix);
		break;
	case DC_SAMPLE_TTS:
		buf_printf(w->out, "{\"t\":\"tts\",\"s\":%u}", value->time);
		break;
	default:
		break;
	}
}

static void put_u32_field(buf_t *b, const char *name, dc_parser_t *parser, dc_field_type_t field)
{
	unsigned int v = 0;
	if (dc_parser_get_field(parser, field, 0, &v) != DC_STATUS_SUCCESS)
		return;
	buf_printf(b, ",\"%s\":%u", name, v);
}

static void put_f64_field(buf_t *b, const char *name, dc_parser_t *parser, dc_field_type_t field)
{
	double v = 0;
	if (dc_parser_get_field(parser, field, 0, &v) != DC_STATUS_SUCCESS)
		return;
	buf_printf(b, ",\"%s\":%.17g", name, v);
}

/* Appends the full RawDive object for a parser. */
static void emit_raw_dive(dc_parser_t *parser, const char *vendor, const char *product, buf_t *b)
{
	buf_put(b, "{\"vendor\":");
	buf_string(b, vendor);
	buf_put(b, ",\"product\":");
	buf_string(b, product);

	dc_datetime_t dt;
	if (dc_parser_get_datetime(parser, &dt) == DC_STATUS_SUCCESS) {
		buf_printf(b,
			",\"datetime\":{\"year\":%d,\"month\":%u,\"day\":%u,\"hour\":%u,"
			"\"minute\":%u,\"second\":%u",
			dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second);
		if (dt.timezone != DC_TIMEZONE_NONE)
			buf_printf(b, ",\"timezone\":%d", dt.timezone);
		buf_put(b, "}");
	}

	put_u32_field(b, "divetime", parser, DC_FIELD_DIVETIME);
	put_f64_field(b, "max_depth", parser, DC_FIELD_MAXDEPTH);
	put_f64_field(b, "mean_depth", parser, DC_FIELD_AVGDEPTH);
	put_f64_field(b, "atmospheric", parser, DC_FIELD_ATMOSPHERIC);
	put_f64_field(b, "temperature_surface", parser, DC_FIELD_TEMPERATURE_SURFACE);
	put_f64_field(b, "temperature_min", parser, DC_FIELD_TEMPERATURE_MINIMUM);

	dc_salinity_t salinity;
	if (dc_parser_get_field(parser, DC_FIELD_SALINITY, 0, &salinity) == DC_STATUS_SUCCESS
		&& salinity.density > 0.0)
		buf_printf(b, ",\"salinity_density\":%.17g", salinity.density);

	dc_divemode_t divemode;
	if (dc_parser_get_field(parser, DC_FIELD_DIVEMODE, 0, &divemode) == DC_STATUS_SUCCESS) {
		buf_put(b, ",\"divemode\":");
		put_divemode(b, divemode);
	}

	dc_decomodel_t decomodel;
	if (dc_parser_get_field(parser, DC_FIELD_DECOMODEL, 0, &decomodel) == DC_STATUS_SUCCESS
		&& (decomodel.params.gf.low > 0 || decomodel.params.gf.high > 0))
		buf_printf(b, ",\"gf\":{\"low\":%u,\"high\":%u}",
			decomodel.params.gf.low, decomodel.params.gf.high);

	dc_event_devinfo_t devinfo;
	if (dc_parser_get_device_info(parser, &devinfo) == DC_STATUS_SUCCESS)
		buf_printf(b,
			",\"info\":{\"model\":%u,\"firmware\":%u,\"serial\":%u,\"hw_id\":%u}",
			devinfo.model, devinfo.firmware, devinfo.serial, devinfo.hw_id);

	unsigned int count = 0;
	dc_parser_get_field(parser, DC_FIELD_GASMIX_COUNT, 0, &count);
	buf_put(b, ",\"gasmixes\":[");
	for (unsigned int i = 0; i < count; i++) {
		dc_gasmix_t mix;
		if (dc_parser_get_field(parser, DC_FIELD_GASMIX, i, &mix) != DC_STATUS_SUCCESS)
			continue;
		if (i)
			buf_put(b, ",");
		buf_printf(b, "{\"oxygen\":%.17g,\"helium\":%.17g,\"usage\":", mix.oxygen, mix.helium);
		put_usage(b, mix.usage);
		buf_put(b, "}");
	}
	buf_put(b, "]");

	count = 0;
	dc_parser_get_field(parser, DC_FIELD_TANK_COUNT, 0, &count);
	buf_put(b, ",\"tanks\":[");
	for (unsigned int i = 0; i < count; i++) {
		dc_tank_t tank;
		if (dc_parser_get_field(parser, DC_FIELD_TANK, i, &tank) != DC_STATUS_SUCCESS)
			continue;
		if (i)
			buf_put(b, ",");
		buf_put(b, "{");
		if (tank.gasmix != DC_GASMIX_UNKNOWN)
			buf_printf(b, "\"gasmix\":%u,", tank.gasmix);
		buf_printf(b,
			"\"volume\":%.17g,\"workpressure\":%.17g,\"beginpressure\":%.17g,\"endpressure\":%.17g}",
			tank.volume, tank.workpressure, tank.beginpressure, tank.endpressure);
	}
	buf_put(b, "]");

	buf_put(b, ",\"samples\":[");
	sample_writer_t writer = { .out = b, .first = 1 };
	dc_parser_samples_foreach(parser, write_sample, &writer);
	buf_put(b, "]}");
}

/* ------------------------------------------------------------------ parser */

static dc_descriptor_t *find_descriptor(dc_context_t *context, dc_iterator_t **out_iterator,
	const char *vendor, const char *product)
{
	dc_iterator_t *iterator = NULL;
	dc_descriptor_t *descriptor = NULL;
	dc_descriptor_t *found = NULL;

	if (dc_descriptor_iterator_new(&iterator, context) != DC_STATUS_SUCCESS)
		return NULL;
	while (dc_iterator_next(iterator, &descriptor) == DC_STATUS_SUCCESS) {
		const char *v = dc_descriptor_get_vendor(descriptor);
		const char *p = dc_descriptor_get_product(descriptor);
		if (v && p && strcmp(v, vendor) == 0 && strcmp(p, product) == 0) {
			found = descriptor;
			break;
		}
		dc_descriptor_free(descriptor);
	}
	dc_iterator_free(iterator);
	if (out_iterator)
		*out_iterator = NULL;
	return found;
}

EMSCRIPTEN_KEEPALIVE
char *benthic_dc_parse(const char *vendor, const char *product,
	const unsigned char *data, unsigned int size)
{
	dc_context_t *context = NULL;
	dc_descriptor_t *descriptor = NULL;
	dc_parser_t *parser = NULL;
	buf_t b = {0};
	char *result = NULL;

	if (dc_context_new(&context) != DC_STATUS_SUCCESS)
		return NULL;
	descriptor = find_descriptor(context, NULL, vendor, product);
	if (!descriptor)
		goto cleanup;
	if (dc_parser_new2(&parser, context, descriptor, data, size) != DC_STATUS_SUCCESS)
		goto cleanup;

	emit_raw_dive(parser, vendor, product, &b);
	result = b.data ? b.data : strdup("{}");
	b.data = NULL;

cleanup:
	if (parser)
		dc_parser_destroy(parser);
	if (descriptor)
		dc_descriptor_free(descriptor);
	dc_context_free(context);
	free(b.data);
	return result;
}

/* ------------------------------------------------------- async transport -- */
/*
 * Each function below forwards to globalThis.benthicHost. Asyncify.handleAsync
 * suspends the wasm stack across the await and resumes it with the result.
 */

EM_JS(int, benthic_js_configure, (unsigned int baudrate, unsigned int databits,
	unsigned int parity, unsigned int stopbits, unsigned int flowcontrol), {
	return Asyncify.handleAsync(async () => {
		try {
			await globalThis.benthicHost.configure(baudrate, databits, parity, stopbits, flowcontrol);
			return 0;
		} catch (error) {
			return -1;
		}
	});
});

EM_JS(int, benthic_js_read, (void *data, unsigned int size, int timeout), {
	return Asyncify.handleAsync(async () => {
		try {
			const chunk = await globalThis.benthicHost.read(size, timeout);
			if (!chunk || chunk.length === 0) return 0;
			const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
			const n = Math.min(bytes.length, size);
			HEAPU8.set(bytes.subarray(0, n), data);
			return n;
		} catch (error) {
			return -1;
		}
	});
});

EM_JS(int, benthic_js_write, (const void *data, unsigned int size), {
	return Asyncify.handleAsync(async () => {
		try {
			const copy = new Uint8Array(HEAPU8.subarray(data, data + size));
			const written = await globalThis.benthicHost.write(copy);
			return typeof written === "number" ? written : size;
		} catch (error) {
			return -1;
		}
	});
});

EM_JS(int, benthic_js_poll, (int timeout), {
	return Asyncify.handleAsync(async () => {
		try {
			return (await globalThis.benthicHost.poll(timeout)) ? 1 : 0;
		} catch (error) {
			return -1;
		}
	});
});

EM_JS(int, benthic_js_available, (), {
	try {
		return globalThis.benthicHost.available ? globalThis.benthicHost.available() : 0;
	} catch (error) {
		return -1;
	}
});

EM_JS(int, benthic_js_sleep, (unsigned int ms), {
	return Asyncify.handleAsync(() => new Promise((resolve) => setTimeout(() => resolve(0), ms)));
});

EM_JS(int, benthic_js_close, (), {
	return Asyncify.handleAsync(async () => {
		try {
			await globalThis.benthicHost.close();
			return 0;
		} catch (error) {
			return -1;
		}
	});
});

EM_JS(void, benthic_js_event, (const char *json), {
	try {
		globalThis.benthicHost.event?.(JSON.parse(UTF8ToString(json)));
	} catch (error) {
		/* ignore: events are advisory */
	}
});

/* Delivers the final JSON back to JS. Asyncify may resume the C function long
 * after the export returned, so callers wait on this callback rather than on
 * the export's return value. */
EM_JS(void, benthic_js_result, (const char *json), {
	try {
		globalThis.benthicHost.result?.(UTF8ToString(json));
	} catch (error) {
		/* ignore */
	}
});

typedef struct {
	int timeout;
} custom_user_t;

static dc_status_t io_set_timeout(void *userdata, int timeout)
{
	((custom_user_t *)userdata)->timeout = timeout;
	return DC_STATUS_SUCCESS;
}

static dc_status_t io_configure(void *userdata, unsigned int baudrate, unsigned int databits,
	dc_parity_t parity, dc_stopbits_t stopbits, dc_flowcontrol_t flowcontrol)
{
	(void)userdata;
	return benthic_js_configure(baudrate, databits, (unsigned int)parity,
		(unsigned int)stopbits, (unsigned int)flowcontrol) == 0
		? DC_STATUS_SUCCESS : DC_STATUS_IO;
}

static dc_status_t io_read(void *userdata, void *data, size_t size, size_t *actual)
{
	int n = benthic_js_read(data, (unsigned int)size, ((custom_user_t *)userdata)->timeout);
	if (n < 0)
		return DC_STATUS_IO;
	*actual = (size_t)n;
	return DC_STATUS_SUCCESS;
}

static dc_status_t io_write(void *userdata, const void *data, size_t size, size_t *actual)
{
	(void)userdata;
	int n = benthic_js_write(data, (unsigned int)size);
	if (n < 0)
		return DC_STATUS_IO;
	*actual = (size_t)n;
	return DC_STATUS_SUCCESS;
}

static dc_status_t io_poll(void *userdata, int timeout)
{
	(void)userdata;
	return benthic_js_poll(timeout) == 1 ? DC_STATUS_SUCCESS : DC_STATUS_TIMEOUT;
}

static dc_status_t io_get_available(void *userdata, size_t *value)
{
	(void)userdata;
	int n = benthic_js_available();
	if (n < 0)
		return DC_STATUS_IO;
	*value = (size_t)n;
	return DC_STATUS_SUCCESS;
}

static dc_status_t io_flush(void *userdata) { (void)userdata; return DC_STATUS_SUCCESS; }
static dc_status_t io_purge(void *userdata, dc_direction_t direction) { (void)userdata; (void)direction; return DC_STATUS_SUCCESS; }
static dc_status_t io_sleep(void *userdata, unsigned int ms) { (void)userdata; benthic_js_sleep(ms); return DC_STATUS_SUCCESS; }
static dc_status_t io_close(void *userdata) { (void)userdata; return benthic_js_close() == 0 ? DC_STATUS_SUCCESS : DC_STATUS_IO; }

static dc_status_t io_set_break(void *userdata, unsigned int value) { (void)userdata; (void)value; return DC_STATUS_UNSUPPORTED; }
static dc_status_t io_set_dtr(void *userdata, unsigned int value) { (void)userdata; (void)value; return DC_STATUS_UNSUPPORTED; }
static dc_status_t io_set_rts(void *userdata, unsigned int value) { (void)userdata; (void)value; return DC_STATUS_UNSUPPORTED; }
static dc_status_t io_get_lines(void *userdata, unsigned int *value) { (void)userdata; (void)value; return DC_STATUS_UNSUPPORTED; }
static dc_status_t io_ioctl(void *userdata, unsigned int request, void *data, size_t size) { (void)userdata; (void)request; (void)data; (void)size; return DC_STATUS_UNSUPPORTED; }

static const dc_custom_cbs_t io_callbacks = {
	.set_timeout = io_set_timeout,
	.set_break = io_set_break,
	.set_dtr = io_set_dtr,
	.set_rts = io_set_rts,
	.get_lines = io_get_lines,
	.get_available = io_get_available,
	.configure = io_configure,
	.poll = io_poll,
	.read = io_read,
	.write = io_write,
	.ioctl = io_ioctl,
	.flush = io_flush,
	.purge = io_purge,
	.sleep = io_sleep,
	.close = io_close,
};

/* ------------------------------------------------------------- download --- */

typedef struct {
	dc_device_t *device;
	const char *vendor;
	const char *product;
	buf_t dives;      /* JSON array contents */
	int first_dive;
	int have_fingerprint;
	unsigned char fingerprint[64];
	unsigned int fingerprint_size;
} download_state_t;

static void event_cb(dc_device_t *device, dc_event_type_t event, const void *data, void *userdata)
{
	(void)device;
	(void)userdata;
	buf_t b = {0};
	switch (event) {
	case DC_EVENT_DEVINFO: {
		const dc_event_devinfo_t *info = data;
		buf_printf(&b, "{\"event\":\"devinfo\",\"model\":%u,\"firmware\":%u,\"serial\":%u,\"hw_id\":%u}",
			info->model, info->firmware, info->serial, info->hw_id);
		break;
	}
	case DC_EVENT_PROGRESS: {
		const dc_event_progress_t *progress = data;
		buf_printf(&b, "{\"event\":\"progress\",\"current\":%u,\"maximum\":%u}",
			progress->current, progress->maximum);
		break;
	}
	case DC_EVENT_WAITING:
		buf_put(&b, "{\"event\":\"waiting\"}");
		break;
	default:
		break;
	}
	if (b.data) {
		benthic_js_event(b.data);
		free(b.data);
	}
}

static int dive_cb(const unsigned char *data, unsigned int size,
	const unsigned char *fingerprint, unsigned int fsize, void *userdata)
{
	download_state_t *state = userdata;
	dc_context_t *context = NULL;
	dc_parser_t *parser = NULL;

	if (!state->have_fingerprint && fingerprint && fsize > 0) {
		state->fingerprint_size = fsize < sizeof(state->fingerprint)
			? fsize : (unsigned int)sizeof(state->fingerprint);
		memcpy(state->fingerprint, fingerprint, state->fingerprint_size);
		state->have_fingerprint = 1;
	}

	dc_device_get_type(state->device); /* no-op, keeps the device referenced */
	if (dc_parser_new(&parser, state->device, data, size) != DC_STATUS_SUCCESS)
		return 1; /* skip unparseable dives, keep going */

	if (!state->first_dive)
		buf_put(&state->dives, ",");
	state->first_dive = 0;
	emit_raw_dive(parser, state->vendor, state->product, &state->dives);
	dc_parser_destroy(parser);
	(void)context;
	return 1;
}

EMSCRIPTEN_KEEPALIVE
void benthic_dc_download(const char *vendor, const char *product, const char *fingerprint_hex)
{
	dc_context_t *context = NULL;
	dc_descriptor_t *descriptor = NULL;
	dc_iostream_t *iostream = NULL;
	dc_device_t *device = NULL;
	buf_t result = {0};
	custom_user_t custom = { .timeout = 0 };
	download_state_t state = {0};
	unsigned char fingerprint[64];
	unsigned int fingerprint_size = 0;
	int status;

	state.vendor = vendor;
	state.product = product;
	state.first_dive = 1;

	if (dc_context_new(&context) != DC_STATUS_SUCCESS) {
		benthic_js_result("{\"error\":\"could not create context\"}");
		return;
	}

	if (dc_custom_open(&iostream, context, DC_TRANSPORT_SERIAL, &io_callbacks, &custom)
		!= DC_STATUS_SUCCESS) {
		buf_put(&result, "{\"error\":\"could not open the transport\"}");
		goto cleanup;
	}

	descriptor = find_descriptor(context, NULL, vendor, product);
	if (!descriptor) {
		buf_put(&result, "{\"error\":\"unsupported dive computer\"}");
		goto cleanup;
	}

	if (dc_device_open(&device, context, descriptor, iostream) != DC_STATUS_SUCCESS) {
		buf_put(&result, "{\"error\":\"could not open the device\"}");
		goto cleanup;
	}
	state.device = device;

	dc_device_set_events(device,
		DC_EVENT_WAITING | DC_EVENT_PROGRESS | DC_EVENT_DEVINFO, event_cb, &state);

	if (fingerprint_hex && fingerprint_hex[0]) {
		fingerprint_size = hex_decode(fingerprint_hex, fingerprint, sizeof(fingerprint));
		if (fingerprint_size > 0)
			dc_device_set_fingerprint(device, fingerprint, fingerprint_size);
	}

	status = dc_device_foreach(device, dive_cb, &state);
	if (status != DC_STATUS_SUCCESS && status != DC_STATUS_DONE) {
		buf_printf(&result, "{\"error\":\"download failed (status %d)\"}", status);
		goto cleanup;
	}

	buf_put(&result, "{\"error\":null,\"fingerprint\":\"");
	if (state.have_fingerprint)
		hex_encode(&result, state.fingerprint, state.fingerprint_size);
	buf_put(&result, "\",\"dives\":[");
	if (state.dives.data)
		buf_putn(&result, state.dives.data, state.dives.len);
	buf_put(&result, "]}");

cleanup:
	if (device)
		dc_device_close(device);
	if (iostream)
		dc_iostream_close(iostream);
	if (descriptor)
		dc_descriptor_free(descriptor);
	dc_context_free(context);
	free(state.dives.data);
	benthic_js_result(result.data ? result.data : "{\"error\":\"unknown failure\"}");
	free(result.data);
}

/*
 * Exercises the async custom iostream end to end (configure, write, sleep,
 * read) without a real device. Used by the Node test to prove the Asyncify
 * round-trip.
 */
EMSCRIPTEN_KEEPALIVE
void benthic_dc_selftest(void)
{
	dc_context_t *context = NULL;
	dc_iostream_t *iostream = NULL;
	custom_user_t custom = { .timeout = 1000 };
	unsigned char rx[16] = {0};
	size_t actual = 0;
	size_t wrote = 0;
	buf_t b = {0};

	if (dc_context_new(&context) != DC_STATUS_SUCCESS) {
		benthic_js_result("{\"error\":\"no context\"}");
		return;
	}
	if (dc_custom_open(&iostream, context, DC_TRANSPORT_SERIAL, &io_callbacks, &custom)
		!= DC_STATUS_SUCCESS) {
		dc_context_free(context);
		benthic_js_result("{\"error\":\"no iostream\"}");
		return;
	}

	dc_iostream_set_timeout(iostream, custom.timeout);
	dc_iostream_configure(iostream, 9600, 8, DC_PARITY_NONE, DC_STOPBITS_ONE, DC_FLOWCONTROL_NONE);
	dc_iostream_write(iostream, "ping", 4, &wrote);
	dc_iostream_sleep(iostream, 5);
	dc_iostream_read(iostream, rx, 4, &actual);
	dc_iostream_close(iostream);
	dc_context_free(context);

	buf_printf(&b, "{\"wrote\":%zu,\"read\":%zu,\"text\":\"", wrote, actual);
	buf_putn(&b, (const char *)rx, actual);
	buf_put(&b, "\"}");
	benthic_js_result(b.data ? b.data : "{}");
	free(b.data);
}

EMSCRIPTEN_KEEPALIVE
void benthic_dc_free(char *ptr)
{
	free(ptr);
}
