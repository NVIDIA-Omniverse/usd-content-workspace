use azure_core::headers::{HeaderName, Headers};

pub const VERSION_ID: HeaderName = HeaderName::from_static("x-ms-version-id");
pub const IS_CURRENT_VERSION: HeaderName = HeaderName::from_static("x-ms-is-current-version");

pub fn is_current_version_from_headers(headers: &Headers) -> azure_core::Result<Option<bool>> {
    headers.get_optional_as(&IS_CURRENT_VERSION)
}

pub fn version_id_from_headers(headers: &Headers) -> azure_core::Result<Option<String>> {
    headers.get_optional_as(&VERSION_ID)
}
