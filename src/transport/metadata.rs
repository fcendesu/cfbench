use serde_json::{Map, Value};

use crate::error::MetadataStructureError;
use crate::results::{ClientLocation, EdgeLocation, NetworkMetadata};

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
