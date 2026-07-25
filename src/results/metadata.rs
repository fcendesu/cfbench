use serde::Serialize;

/// Whether post-measurement network metadata was collected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MetadataStatus {
    Available,
    #[default]
    Unavailable,
    Disabled,
}

/// Public client-network metadata reported by Cloudflare.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct NetworkMetadata {
    pub public_ip: Option<String>,
    pub asn: Option<u32>,
    pub as_organization: Option<String>,
    pub client_location: ClientLocation,
    pub edge: EdgeLocation,
}

/// Approximate public client location reported by Cloudflare.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ClientLocation {
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Cloudflare edge location that served the metadata response.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EdgeLocation {
    pub colo: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}
