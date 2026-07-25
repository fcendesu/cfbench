use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::error::MetadataStructureError;
use crate::results::{ClientLocation, EdgeLocation, NetworkMetadata};

pub(crate) enum MetadataDecodeError {
    Json(serde_json::Error),
    Structure(MetadataStructureError),
}

#[derive(Deserialize)]
struct MetadataWire<'a> {
    #[serde(borrow, rename = "clientIp")]
    client_ip: Option<&'a RawValue>,
    #[serde(borrow)]
    asn: Option<&'a RawValue>,
    #[serde(borrow, rename = "asOrganization")]
    as_organization: Option<&'a RawValue>,
    #[serde(borrow)]
    country: Option<&'a RawValue>,
    #[serde(borrow)]
    city: Option<&'a RawValue>,
    #[serde(borrow)]
    region: Option<&'a RawValue>,
    #[serde(borrow, rename = "postalCode")]
    postal_code: Option<&'a RawValue>,
    #[serde(borrow)]
    latitude: Option<&'a RawValue>,
    #[serde(borrow)]
    longitude: Option<&'a RawValue>,
    #[serde(borrow)]
    colo: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct EdgeWire<'a> {
    #[serde(borrow)]
    iata: Option<&'a RawValue>,
    #[serde(borrow)]
    cca2: Option<&'a RawValue>,
    #[serde(borrow)]
    region: Option<&'a RawValue>,
    #[serde(borrow)]
    city: Option<&'a RawValue>,
    #[serde(borrow)]
    lat: Option<&'a RawValue>,
    #[serde(borrow)]
    lon: Option<&'a RawValue>,
}

/// Decodes only selected metadata fields from one already body-bounded document.
///
/// `RawValue` capture and Serde's ignored-value path both use serde_json's
/// iterative value scanner. Deep additive fields and deep wrong optional leaf
/// types therefore never become a recursive `Value` tree or consume call stack.
pub(crate) fn metadata_from_slice(body: &[u8]) -> Result<NetworkMetadata, MetadataDecodeError> {
    let root: &RawValue = serde_json::from_slice(body).map_err(MetadataDecodeError::Json)?;
    if first_non_whitespace(root.get()) != Some(b'{') {
        return Err(MetadataDecodeError::Structure(
            MetadataStructureError::TopLevelNotObject,
        ));
    }

    let wire: MetadataWire<'_> =
        serde_json::from_str(root.get()).map_err(MetadataDecodeError::Json)?;
    let edge = edge_from_raw(wire.colo);
    Ok(NetworkMetadata {
        public_ip: raw_string(wire.client_ip),
        asn: raw_u32(wire.asn),
        as_organization: raw_string(wire.as_organization),
        client_location: ClientLocation {
            country_code: raw_string(wire.country),
            city: raw_string(wire.city),
            region: raw_string(wire.region),
            postal_code: raw_string(wire.postal_code),
            latitude: raw_coordinate(wire.latitude),
            longitude: raw_coordinate(wire.longitude),
        },
        edge,
    })
}

/// Maps Cloudflare's additive `/meta` response one optional leaf at a time.
pub fn metadata_from_value(value: Value) -> Result<NetworkMetadata, MetadataStructureError> {
    let Value::Object(root) = value else {
        return Err(MetadataStructureError::TopLevelNotObject);
    };
    let edge = root.get("colo").and_then(Value::as_object);

    Ok(NetworkMetadata {
        public_ip: string_leaf(&root, "clientIp"),
        asn: root
            .get("asn")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        as_organization: string_leaf(&root, "asOrganization"),
        client_location: ClientLocation {
            country_code: string_leaf(&root, "country"),
            city: string_leaf(&root, "city"),
            region: string_leaf(&root, "region"),
            postal_code: string_leaf(&root, "postalCode"),
            latitude: coordinate_leaf(&root, "latitude"),
            longitude: coordinate_leaf(&root, "longitude"),
        },
        edge: EdgeLocation {
            colo: edge.and_then(|object| string_leaf(object, "iata")),
            country_code: edge.and_then(|object| string_leaf(object, "cca2")),
            region: edge.and_then(|object| string_leaf(object, "region")),
            city: edge.and_then(|object| string_leaf(object, "city")),
            latitude: edge.and_then(|object| coordinate_leaf(object, "lat")),
            longitude: edge.and_then(|object| coordinate_leaf(object, "lon")),
        },
    })
}

fn string_leaf(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToOwned::to_owned)
}

fn coordinate_leaf(object: &Map<String, Value>, key: &str) -> Option<f64> {
    let value = match object.get(key)? {
        Value::Number(number) => number.as_f64()?,
        Value::String(value) => value.parse().ok()?,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

fn edge_from_raw(raw: Option<&RawValue>) -> EdgeLocation {
    let Some(raw) = raw else {
        return EdgeLocation::default();
    };
    if first_non_whitespace(raw.get()) != Some(b'{') {
        return EdgeLocation::default();
    }
    let Ok(edge) = serde_json::from_str::<EdgeWire<'_>>(raw.get()) else {
        return EdgeLocation::default();
    };

    EdgeLocation {
        colo: raw_string(edge.iata),
        country_code: raw_string(edge.cca2),
        region: raw_string(edge.region),
        city: raw_string(edge.city),
        latitude: raw_coordinate(edge.lat),
        longitude: raw_coordinate(edge.lon),
    }
}

fn raw_string(raw: Option<&RawValue>) -> Option<String> {
    let raw = raw?;
    (first_non_whitespace(raw.get()) == Some(b'"'))
        .then(|| serde_json::from_str(raw.get()).ok())
        .flatten()
}

fn raw_u32(raw: Option<&RawValue>) -> Option<u32> {
    let raw = raw?;
    first_non_whitespace(raw.get())?
        .is_ascii_digit()
        .then(|| serde_json::from_str(raw.get()).ok())
        .flatten()
}

fn raw_coordinate(raw: Option<&RawValue>) -> Option<f64> {
    let raw = raw?;
    let value = match first_non_whitespace(raw.get())? {
        b'"' => serde_json::from_str::<String>(raw.get())
            .ok()?
            .parse::<f64>()
            .ok()?,
        b'-' | b'0'..=b'9' => serde_json::from_str::<f64>(raw.get()).ok()?,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

fn first_non_whitespace(json: &str) -> Option<u8> {
    json.bytes().find(|byte| !byte.is_ascii_whitespace())
}
