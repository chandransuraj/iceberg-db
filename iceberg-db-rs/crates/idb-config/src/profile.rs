//! Catalog profile presets (aligned with Java `CatalogProfile`).



use std::collections::BTreeMap;



const VENDED_CREDENTIALS: &str = "vended-credentials";



/// PAT secret from `credential` (handles mistaken `user:pat` form).

pub fn snowflake_pat_secret(credential: &str) -> &str {

    credential

        .split_once(':')

        .map(|(_, pat)| pat)

        .unwrap_or(credential)

}



/// Validate REST catalog props after profile expansion and env substitution.

pub fn validate_rest_props(

    catalog_name: &str,

    props: &BTreeMap<String, String>,

    profile: Option<&str>,

) -> Result<(), String> {

    let uri = props

        .get("uri")

        .filter(|s| !s.is_empty())

        .ok_or_else(|| format!("catalog '{catalog_name}': missing `uri`"))?;



    if uri.contains('<') || uri.contains('>') {

        return Err(format!(

            "catalog '{catalog_name}': `uri` still contains placeholders ({uri}). \

Set your Snowflake account, e.g. uri: xy12345.us-east-1 or \

uri: https://xy12345.us-east-1.snowflakecomputing.com/polaris/api/catalog"

        ));

    }



    if !uri.starts_with("http://") && !uri.starts_with("https://") {

        return Err(format!(

            "catalog '{catalog_name}': `uri` must start with http:// or https:// after profile expansion (got {uri}). \

Use account shorthand like xy12345.us-east-1 or a full https://…snowflakecomputing.com/polaris/api/catalog URL"

        ));

    }



    if props.get("token").is_none_or(|t| t.is_empty())

        && props.get("credential").is_none_or(|c| c.is_empty())

    {

        return Err(format!(

            "catalog '{catalog_name}': set `token: ${{SNOWFLAKE_ACCESS_TOKEN}}` to your Snowflake PAT"

        ));

    }



    if is_snowflake_horizon_profile(profile, props.get("uri").map(String::as_str)) {

        let scope = props.get("scope").or_else(|| props.get("oauth2-scope"));

        if scope.is_none_or(|s| s.is_empty() || !s.starts_with("session:role:")) {

            return Err(format!(

                "catalog '{catalog_name}': snowflake-horizon requires \

`scope: session:role:<ROLE>` matching the PAT ROLE_RESTRICTION (e.g. session:role:SYSADMIN)"

            ));

        }

        if props.get("oauth2-client-id").is_none_or(|s| s.is_empty()) {

            return Err(format!(

                "catalog '{catalog_name}': snowflake-horizon requires `username` (OAuth client_id), \

e.g. username: CHANDRANSURAJ"

            ));

        }

        let cred = props

            .get("credential")

            .filter(|s| !s.is_empty())

            .or_else(|| props.get("token").filter(|s| !s.is_empty()))

            .map(String::as_str)

            .unwrap_or("");

        let pat = snowflake_pat_secret(cred);

        if pat.len() < 32 {

            return Err(format!(

                "catalog '{catalog_name}': SNOWFLAKE_ACCESS_TOKEN is missing or not a PAT \

({pat_len} chars after resolving ${{SNOWFLAKE_ACCESS_TOKEN}}; expected ~200 for a JWT PAT). \

In the same PowerShell window run:\n  \

$env:SNOWFLAKE_ACCESS_TOKEN = '<paste PAT from Snowsight>'\n  \

$env:SNOWFLAKE_ACCESS_TOKEN.Length",

                pat_len = pat.len()

            ));

        }

    }



    Ok(())

}



pub fn is_snowflake_horizon_profile(profile: Option<&str>, uri: Option<&str>) -> bool {
    if profile
        .map(str::to_ascii_lowercase)
        .as_deref()
        .is_some_and(|p| p == "snowflake-horizon")
    {
        return true;
    }
    uri.is_some_and(|u| u.contains("snowflakecomputing.com") && u.contains("polaris"))
}



/// Apply a named profile and normalize REST property names for `iceberg-catalog-rest`.

pub fn apply_profile(profile: Option<&str>, props: &mut BTreeMap<String, String>) {

    normalize_rest_headers(props);

    match profile.map(str::to_ascii_lowercase).as_deref() {

        Some("snowflake-horizon") => apply_snowflake_horizon(props),

        _ => {}

    }

}



fn normalize_rest_headers(props: &mut BTreeMap<String, String>) {

    let java_headers: Vec<(String, String)> = props

        .iter()

        .filter_map(|(k, v)| {

            k.strip_prefix("rest.headers.")

                .map(|name| (format!("header.{name}"), v.clone()))

        })

        .collect();

    for (k, v) in java_headers {

        props.entry(k).or_insert(v);

    }

}



fn apply_snowflake_horizon(props: &mut BTreeMap<String, String>) {

    if let Some(username) = props.remove("username") {

        props

            .entry("oauth2-client-id".into())

            .or_insert(username);

    }



    if let Some(uri) = props.get("uri").cloned() {

        if !uri.contains("snowflakecomputing.com") && !uri.starts_with("http") {

            props.insert(

                "uri".into(),

                format!("https://{uri}.snowflakecomputing.com/polaris/api/catalog"),

            );

        }

    }



    if let Some(pat) = props.remove("token") {

        props.insert("credential".into(), pat);

    }



    if let Some(scope) = props.get("scope").cloned() {

        props.entry("oauth2-scope".into()).or_insert(scope);

    }



    if let Some(uri) = props.get("uri").cloned() {

        let oauth = if uri.ends_with('/') {

            format!("{uri}v1/oauth/tokens")

        } else {

            format!("{uri}/v1/oauth/tokens")

        };

        props

            .entry("oauth2-server-uri".into())

            .or_insert(oauth);

    }

}



/// Header value for Snowflake loadTable (not for OAuth).

pub fn snowflake_vended_credentials_header() -> (&'static str, &'static str) {

    ("X-Iceberg-Access-Delegation", VENDED_CREDENTIALS)

}



#[cfg(test)]

mod tests {

    use super::*;



    #[test]

    fn snowflake_horizon_expands_uri_and_oauth_client() {

        let mut props = BTreeMap::from([

            ("uri".into(), "xy12345.us-east-1".into()),

            ("warehouse".into(), "ANALYTICS_DB".into()),

            ("token".into(), "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.pat-secret-part".into()),

            ("username".into(), "CHANDRANSURAJ".into()),

            ("scope".into(), "session:role:SYSADMIN".into()),

        ]);

        apply_profile(Some("snowflake-horizon"), &mut props);

        assert_eq!(

            props.get("uri").map(String::as_str),

            Some("https://xy12345.us-east-1.snowflakecomputing.com/polaris/api/catalog")

        );

        assert!(props.get("credential").unwrap().starts_with("eyJ"));

        assert_eq!(

            props.get("oauth2-client-id").map(String::as_str),

            Some("CHANDRANSURAJ")

        );

        assert!(validate_rest_props("sf", &props, Some("snowflake-horizon")).is_ok());

    }



    #[test]

    fn snowflake_rejects_username_only_credential() {

        let props = BTreeMap::from([

            ("uri".into(), "https://xy.snowflakecomputing.com/polaris/api/catalog".into()),

            ("credential".into(), "CHANDRANSURAJ:".into()),

            ("oauth2-client-id".into(), "CHANDRANSURAJ".into()),

            ("scope".into(), "session:role:SYSADMIN".into()),

        ]);

        assert!(validate_rest_props("sf", &props, Some("snowflake-horizon")).is_err());

    }

}


