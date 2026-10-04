mod compare;
#[cfg(feature = "divecomputer")]
mod device_download;
#[cfg(target_arch = "wasm32")]
mod device_download_web;
mod dive_detail;
mod dive_list;
mod filter_bar;
mod import_export;
mod info_tip;
mod map;
mod planner;
mod preferences;
mod profile;
mod profile_editor;
mod selection_bar;
mod sync_dialog;
mod toolbar;
mod trips_dialog;
mod welcome;

pub use compare::CompareDialog;
#[cfg(feature = "divecomputer")]
pub use device_download::DeviceDownloadDialog;
#[cfg(target_arch = "wasm32")]
pub use device_download_web::DeviceDownloadWebDialog;
pub use dive_detail::DiveDetail;
pub use dive_list::DiveList;
pub use filter_bar::FilterBar;
pub use import_export::ImportExport;
pub use info_tip::InfoTip;
pub use map::{LocationPickerDialog, MapDialog, MapSite, MapView};
pub use planner::PlannerDialog;
pub use preferences::PreferencesDialog;
pub use profile::DiveProfile;
pub use profile_editor::{profile_bounds, ProfileEditorDialog};
pub use selection_bar::SelectionBar;
pub use sync_dialog::SyncDialog;
pub use toolbar::Toolbar;
pub use trips_dialog::TripsDialog;
pub use welcome::WelcomeDialog;
