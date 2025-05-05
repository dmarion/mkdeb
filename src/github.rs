use crate::debug;

pub struct GithubRelease {
    pub tag: String,
    pub tarball_url: String,
    pub version: String,
}

fn extract_deb_version(tag: &str, published_at: Option<&str>) -> String {
    if tag.starts_with('v') && tag.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
        tag.trim_start_matches('v').to_string()
    } else if tag.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        tag.to_string()
    } else {
        published_at
            .map(|d| d.replace(['-', ':', 'T', 'Z'], ""))
            .unwrap_or_else(|| "0.0.0".to_string())
    }
}

pub async fn find_release(repo: &str, version: Option<&str>) -> Option<GithubRelease> {
    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/releases", repo);
    debug!("Fetching {}...", url);
    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "mkdeb/0.1 (https://github.com/youruser/mkdeb)",
        )
        .send()
        .await
        .ok()?;

    let releases: serde_json::Value = response.json().await.ok()?;
    let releases = releases.as_array()?;

    for release in releases {
        let tag = release.get("tag_name")?.as_str()?;
        let published_at = release.get("published_at").and_then(|d| d.as_str());
        let rel_ver = extract_deb_version(tag, published_at);
        debug!(
            "considering tag: {} rel_ver: {} published: {:#?}",
            tag, rel_ver, published_at
        );
        if version.is_none() || version == Some(rel_ver.as_str()) {
            let tarball_url = release.get("tarball_url")?.as_str()?.to_string();

            return Some(GithubRelease {
                tag: tag.to_string(),
                tarball_url,
                version: rel_ver,
            });
        }
    }

    debug!("no releases found, trying with tags");

    let tags_url = format!("https://api.github.com/repos/{}/tags", repo);
    debug!("Fetching {}...", tags_url);
    let tag_response = client
        .get(&tags_url)
        .header(
            "User-Agent",
            "mkdeb/0.1 (https://github.com/youruser/mkdeb)",
        )
        .send()
        .await
        .ok()?;

    let tags: serde_json::Value = tag_response.json().await.ok()?;
    let tags = tags.as_array()?;

    for tag_obj in tags {
        let tag_name = tag_obj.get("name")?.as_str()?;
        let rel_ver = extract_deb_version(tag_name, None);
        if version.is_none() || version == Some(rel_ver.as_str()) {
            let tarball_url = format!("https://api.github.com/repos/{}/tarball/{}", repo, tag_name);

            return Some(GithubRelease {
                tag: tag_name.to_string(),
                tarball_url,
                version: rel_ver,
            });
        }
    }

    None
}
