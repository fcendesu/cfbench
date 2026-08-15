use std::{collections::HashMap, fs};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct IssueForm {
    labels: Vec<String>,
    body: Vec<FormElement>,
}

#[derive(Debug, Deserialize)]
struct FormElement {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    attributes: HashMap<String, serde_yaml::Value>,
    #[serde(default)]
    validations: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct TemplateConfig {
    blank_issues_enabled: bool,
    contact_links: Vec<ContactLink>,
}

#[derive(Debug, Deserialize)]
struct ContactLink {
    name: String,
    url: String,
    about: String,
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &str) -> T {
    let source = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    serde_yaml::from_str(&source).unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

fn required_ids(form: &IssueForm) -> Vec<&str> {
    form.body
        .iter()
        .filter(|element| element.validations.get("required") == Some(&true))
        .filter_map(|element| element.id.as_deref())
        .collect()
}

fn privacy_notice_index(form: &IssueForm) -> usize {
    form.body
        .iter()
        .position(|element| {
            element.kind == "markdown"
                && element.attributes.values().any(|value| {
                    value.as_str().is_some_and(|text| {
                        [
                            "public IP",
                            "ISP",
                            "network name",
                            "ASN",
                            "edge location",
                            "live metadata",
                        ]
                        .iter()
                        .all(|term| text.contains(term))
                    })
                })
        })
        .expect("privacy notice with every sensitive data category")
}

#[test]
fn bug_form_collects_reproducible_context_without_requesting_private_output() {
    let form: IssueForm = read_yaml(".github/ISSUE_TEMPLATE/bug_report.yml");

    assert_eq!(form.labels, ["bug"]);
    let required = required_ids(&form);
    for id in [
        "cfbench_version",
        "operating_system",
        "architecture",
        "flags",
        "reproduction",
        "expected_behavior",
        "actual_behavior",
    ] {
        assert!(required.contains(&id), "missing required bug field {id}");
    }

    let privacy = privacy_notice_index(&form);
    let actual = form
        .body
        .iter()
        .position(|element| element.id.as_deref() == Some("actual_behavior"))
        .expect("actual behavior field");
    assert!(
        privacy < actual,
        "privacy notice must precede output fields"
    );
}

#[test]
fn feature_form_collects_use_case_and_compatibility_impact() {
    let form: IssueForm = read_yaml(".github/ISSUE_TEMPLATE/feature_request.yml");

    assert_eq!(form.labels, ["enhancement"]);
    let required = required_ids(&form);
    for id in [
        "use_case",
        "proposed_behavior",
        "alternatives",
        "compatibility_impact",
    ] {
        assert!(
            required.contains(&id),
            "missing required feature field {id}"
        );
    }
    privacy_notice_index(&form);
}

#[test]
fn issue_template_config_keeps_blank_issues_and_routes_security_privately() {
    let config: TemplateConfig = read_yaml(".github/ISSUE_TEMPLATE/config.yml");

    assert!(config.blank_issues_enabled);
    let security = config
        .contact_links
        .iter()
        .find(|link| link.name.to_ascii_lowercase().contains("security"))
        .expect("security contact link");
    assert!(security.url.ends_with("/SECURITY.md"));
    assert!(security.about.to_ascii_lowercase().contains("privately"));
    assert!(security.about.to_ascii_lowercase().contains("not public"));
}
