mod support;

use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::error::MetadataStructureError;
use cfbench::error::TransportError;
use cfbench::transport::{ReqwestTransport, metadata_from_value};
use serde_json::json;
use support::{FixtureServer, ResponsePlan};
use tokio_util::sync::CancellationToken;

const METADATA_BODY_LIMIT: usize = 65_536;

fn config(ip_mode: IpMode, timeout: Duration) -> RunConfig {
    RunConfig {
        ip_mode,
        request_timeout: timeout,
        ..RunConfig::default()
    }
}

fn transport_for(server: &FixtureServer, timeout: Duration) -> ReqwestTransport {
    ReqwestTransport::with_base_url(config(IpMode::V4Only, timeout), server.url())
        .expect("fixture transport")
}

fn metadata_body_of_len(len: usize) -> Vec<u8> {
    const PREFIX: &[u8] = b"{\"padding\":\"";
    const SUFFIX: &[u8] = b"\"}";
    assert!(len >= PREFIX.len() + SUFFIX.len());
    let mut body = Vec::with_capacity(len);
    body.extend_from_slice(PREFIX);
    body.resize(len - SUFFIX.len(), b'x');
    body.extend_from_slice(SUFFIX);
    assert_eq!(body.len(), len);
    body
}

#[test]
fn maps_cloudflare_meta_and_rejects_only_invalid_leaves() {
    let metadata = metadata_from_value(json!({
        "clientIp": "2a02:ff0::1",
        "asn": 12735,
        "asOrganization": "TurkNet",
        "country": "TR",
        "city": "Istanbul",
        "region": "Istanbul",
        "postalCode": "34096",
        "latitude": "41.01384",
        "longitude": {},
        "unknown": true,
        "colo": {
            "iata": "IST",
            "lat": 41.262222,
            "lon": "NaN",
            "cca2": "TR",
            "region": "Europe",
            "city": "Arnavutkoy"
        }
    }))
    .unwrap();

    assert_eq!(metadata.public_ip.as_deref(), Some("2a02:ff0::1"));
    assert_eq!(metadata.asn, Some(12735));
    assert_eq!(metadata.as_organization.as_deref(), Some("TurkNet"));
    assert_eq!(metadata.client_location.country_code.as_deref(), Some("TR"));
    assert_eq!(metadata.client_location.city.as_deref(), Some("Istanbul"));
    assert_eq!(metadata.client_location.region.as_deref(), Some("Istanbul"));
    assert_eq!(
        metadata.client_location.postal_code.as_deref(),
        Some("34096")
    );
    assert_eq!(metadata.client_location.latitude, Some(41.01384));
    assert_eq!(metadata.client_location.longitude, None);
    assert_eq!(metadata.edge.colo.as_deref(), Some("IST"));
    assert_eq!(metadata.edge.country_code.as_deref(), Some("TR"));
    assert_eq!(metadata.edge.region.as_deref(), Some("Europe"));
    assert_eq!(metadata.edge.city.as_deref(), Some("Arnavutkoy"));
    assert_eq!(metadata.edge.latitude, Some(41.262222));
    assert_eq!(metadata.edge.longitude, None);
}

#[test]
fn coordinate_mapping_accepts_finite_numbers_and_strings_per_leaf() {
    let metadata = metadata_from_value(json!({
        "latitude": -33.8688,
        "longitude": "151.2093",
        "colo": {
            "lat": "1.25",
            "lon": -2.5
        }
    }))
    .unwrap();

    assert_eq!(metadata.client_location.latitude, Some(-33.8688));
    assert_eq!(metadata.client_location.longitude, Some(151.2093));
    assert_eq!(metadata.edge.latitude, Some(1.25));
    assert_eq!(metadata.edge.longitude, Some(-2.5));
}

#[test]
fn missing_null_wrong_and_nonfinite_leaves_map_to_none() {
    let metadata = metadata_from_value(json!({
        "clientIp": 4,
        "asn": 4_294_967_296_u64,
        "asOrganization": false,
        "country": null,
        "city": [],
        "postalCode": {},
        "latitude": "inf",
        "longitude": null,
        "colo": {
            "iata": 123,
            "lat": "-Infinity",
            "lon": [],
            "cca2": null,
            "city": false
        }
    }))
    .unwrap();

    assert_eq!(metadata.public_ip, None);
    assert_eq!(metadata.asn, None);
    assert_eq!(metadata.as_organization, None);
    assert_eq!(metadata.client_location.country_code, None);
    assert_eq!(metadata.client_location.city, None);
    assert_eq!(metadata.client_location.region, None);
    assert_eq!(metadata.client_location.postal_code, None);
    assert_eq!(metadata.client_location.latitude, None);
    assert_eq!(metadata.client_location.longitude, None);
    assert_eq!(metadata.edge.colo, None);
    assert_eq!(metadata.edge.country_code, None);
    assert_eq!(metadata.edge.region, None);
    assert_eq!(metadata.edge.city, None);
    assert_eq!(metadata.edge.latitude, None);
    assert_eq!(metadata.edge.longitude, None);
}

#[test]
fn non_object_metadata_is_a_typed_structural_error() {
    let error = metadata_from_value(json!([{"clientIp": "192.0.2.1"}])).unwrap_err();

    assert!(matches!(error, MetadataStructureError::TopLevelNotObject));
}

#[tokio::test]
async fn fetches_chunked_metadata_with_referer_only_and_identity_encoding() {
    let body = serde_json::to_vec(&json!({
        "clientIp": "2a02:ff0::1",
        "asn": 12735,
        "colo": { "iata": "IST" }
    }))
    .unwrap();
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body,
        chunk_bytes: 7,
        chunk_delay: Duration::from_millis(1),
    })
    .await;
    let transport =
        ReqwestTransport::with_base_url(RunConfig::default(), fixture.url_with_test_context())
            .unwrap();

    let metadata = transport
        .metadata(&CancellationToken::new())
        .await
        .expect("bounded metadata response");

    assert_eq!(metadata.public_ip.as_deref(), Some("2a02:ff0::1"));
    assert_eq!(metadata.asn, Some(12735));
    assert_eq!(metadata.edge.colo.as_deref(), Some("IST"));
    let requests = fixture.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/meta");
    assert_eq!(requests[0].referer, Some(format!("{}/", fixture.url())));
    assert_eq!(requests[0].origin, None);
    assert_eq!(requests[0].authorization, None);
    assert_eq!(requests[0].accept_encoding.as_deref(), Some("identity"));
    assert!(fixture.response_chunk_count() > 1);
}

#[tokio::test]
async fn metadata_keeps_valid_fields_when_known_coordinate_is_out_of_range() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: br#"{"clientIp":"192.0.2.1","latitude":1e400}"#.to_vec(),
        chunk_bytes: 9,
        chunk_delay: Duration::ZERO,
    })
    .await;

    let metadata = transport_for(&fixture, Duration::from_secs(1))
        .metadata(&CancellationToken::new())
        .await
        .expect("out-of-range coordinate affects only its selected leaf");

    assert_eq!(metadata.public_ip.as_deref(), Some("192.0.2.1"));
    assert_eq!(metadata.client_location.latitude, None);
    assert_eq!(fixture.request_count(), 1);
    assert!(fixture.response_chunk_count() > 1);
}

#[tokio::test]
async fn metadata_ignores_out_of_range_number_in_unknown_field() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: br#"{"clientIp":"192.0.2.1","unknownFutureField":1e400}"#.to_vec(),
        chunk_bytes: 11,
        chunk_delay: Duration::ZERO,
    })
    .await;

    let metadata = transport_for(&fixture, Duration::from_secs(1))
        .metadata(&CancellationToken::new())
        .await
        .expect("out-of-range unknown field is ignored");

    assert_eq!(metadata.public_ip.as_deref(), Some("192.0.2.1"));
    assert_eq!(metadata.client_location.latitude, None);
    assert_eq!(metadata.client_location.longitude, None);
    assert_eq!(fixture.request_count(), 1);
    assert!(fixture.response_chunk_count() > 1);
}

#[tokio::test]
async fn accepts_metadata_body_at_exact_limit_in_chunks() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: metadata_body_of_len(METADATA_BODY_LIMIT),
        chunk_bytes: 1_003,
        chunk_delay: Duration::ZERO,
    })
    .await;

    let metadata = transport_for(&fixture, Duration::from_secs(2))
        .metadata(&CancellationToken::new())
        .await
        .expect("exactly 64 KiB metadata body");

    assert_eq!(metadata.public_ip, None);
    assert_eq!(fixture.request_count(), 1);
    assert!(fixture.response_chunk_count() > 1);
}

#[tokio::test]
async fn rejects_metadata_body_one_byte_over_limit_without_retrying() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: metadata_body_of_len(METADATA_BODY_LIMIT + 1),
        chunk_bytes: 1_003,
        chunk_delay: Duration::ZERO,
    })
    .await;

    let error = transport_for(&fixture, Duration::from_secs(2))
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        TransportError::MetadataBodyTooLarge {
            limit: METADATA_BODY_LIMIT,
            ..
        }
    ));
    assert_eq!(error.payload_bytes(), 0);
    assert_eq!(fixture.request_count(), 1);
    assert!(
        error
            .to_string()
            .contains(&format!("{}/meta", fixture.url()))
    );
}

#[tokio::test]
async fn malformed_json_and_non_object_json_are_distinct_typed_errors() {
    let malformed = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: b"{not-json".to_vec(),
        chunk_bytes: 2,
        chunk_delay: Duration::ZERO,
    })
    .await;
    let malformed_error = transport_for(&malformed, Duration::from_secs(1))
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        malformed_error,
        TransportError::MetadataJson { .. }
    ));
    assert_eq!(malformed_error.payload_bytes(), 0);

    let non_object = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: b"[]".to_vec(),
        chunk_bytes: 1,
        chunk_delay: Duration::ZERO,
    })
    .await;
    let shape_error = transport_for(&non_object, Duration::from_secs(1))
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        shape_error,
        TransportError::MetadataStructure {
            source: MetadataStructureError::TopLevelNotObject,
            ..
        }
    ));
    assert_eq!(shape_error.payload_bytes(), 0);
}

#[tokio::test]
async fn metadata_http_status_fails_once_with_redacted_endpoint() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 503,
        body: b"service unavailable".to_vec(),
        chunk_bytes: 4,
        chunk_delay: Duration::ZERO,
    })
    .await;
    let transport =
        ReqwestTransport::with_base_url(RunConfig::default(), fixture.url_with_test_context())
            .unwrap();

    let error = transport
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        TransportError::HttpStatus {
            status: 503,
            payload_bytes: 0,
            ..
        }
    ));
    let message = error.to_string();
    assert!(message.contains(&format!("{}/meta", fixture.url())));
    assert!(!message.contains("user"));
    assert!(!message.contains("secret"));
    assert!(!message.contains("query"));
    assert_eq!(fixture.request_count(), 1);
}

#[tokio::test]
async fn metadata_distinguishes_header_and_body_timeouts_with_zero_payload_usage() {
    let headers = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let header_error = transport_for(&headers, Duration::from_millis(30))
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        header_error,
        TransportError::HeaderTimeout {
            payload_bytes: 0,
            ..
        }
    ));

    let body = FixtureServer::start(ResponsePlan::StallBody).await;
    let body_error = transport_for(&body, Duration::from_millis(30))
        .metadata(&CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        body_error,
        TransportError::BodyTimeout {
            payload_bytes: 0,
            ..
        }
    ));
}

#[tokio::test]
async fn cancellation_preempts_metadata_headers_and_body_without_payload_usage() {
    let headers = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let header_cancellation = CancellationToken::new();
    let task_cancellation = header_cancellation.clone();
    let transport = transport_for(&headers, Duration::from_secs(30));
    let task = tokio::spawn(async move { transport.metadata(&task_cancellation).await });
    headers.wait_until_stalled().await;
    header_cancellation.cancel();
    let header_error = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("header cancellation completes promptly")
        .expect("metadata header task joins")
        .unwrap_err();
    assert!(matches!(
        header_error,
        TransportError::Cancelled { payload_bytes: 0 }
    ));

    let body = FixtureServer::start(ResponsePlan::StallBody).await;
    let body_cancellation = CancellationToken::new();
    let task_cancellation = body_cancellation.clone();
    let transport = transport_for(&body, Duration::from_secs(30));
    let task = tokio::spawn(async move { transport.metadata(&task_cancellation).await });
    body.wait_until_stalled().await;
    body_cancellation.cancel();
    let body_error = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("body cancellation completes promptly")
        .expect("metadata body task joins")
        .unwrap_err();
    assert!(matches!(
        body_error,
        TransportError::Cancelled { payload_bytes: 0 }
    ));
}

#[tokio::test]
async fn metadata_obeys_strict_ip_family_policy() {
    let fixture = FixtureServer::start(ResponsePlan::Metadata {
        status: 200,
        body: b"{}".to_vec(),
        chunk_bytes: 1,
        chunk_delay: Duration::ZERO,
    })
    .await;

    ReqwestTransport::with_base_url(
        config(IpMode::V4Only, Duration::from_secs(1)),
        fixture.url(),
    )
    .unwrap()
    .metadata(&CancellationToken::new())
    .await
    .expect("IPv4-only metadata request");

    let error = ReqwestTransport::with_base_url(
        config(IpMode::V6Only, Duration::from_millis(100)),
        fixture.url(),
    )
    .unwrap()
    .metadata(&CancellationToken::new())
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        TransportError::Request {
            payload_bytes: 0,
            ..
        }
    ));
    assert_eq!(fixture.request_count(), 1);
}
