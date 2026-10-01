mod command_palette;
mod compare;
#[cfg(feature = "divecomputer")]
mod device_download;
#[cfg(target_arch = "wasm32")]
mod device_download_web;
mod dive_detail;
mod dive_list;
mod filter_bar;
mod import_export;
mod map;
mod planner;
mod preferences;
mod profile;
mod sync_dialog;
mod toolbar;
mod trips_dialog;

pub use command_palette::CommandPalette;
pub use compare::CompareDialog;
#[cfg(feature = "divecomputer")]
pub use device_download::DeviceDownloadDialog;
#[cfg(target_arch = "wasm32")]
pub use device_download_web::DeviceDownloadWebDialog;
pub use dive_detail::DiveDetail;
pub use dive_list::DiveList;
pub use filter_bar::FilterBar;
pub use import_export::ImportExport;
pub use map::{MapDialog, MapSite, MapView};
pub use planner::PlannerDialog;
pub use preferences::PreferencesDialog;
pub use profile::DiveProfile;
pub use sync_dialog::SyncDialog;
pub use toolbar::Toolbar;
pub use trips_dialog::TripsDialog;
