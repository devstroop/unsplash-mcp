use anyhow::Result;
use serde_json::Value;

use crate::browser::SharedBrowser;

/// Percent-encode a query-parameter value (spaces as %20, not +).
fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// Normalize a topic/category name to an Unsplash topic slug.
fn topic_slug(category: &str) -> String {
    let slug = category.trim().to_lowercase().replace(' ', "-");
    enc(&slug)
}

fn as_results(value: Value) -> Value {
    // napi search endpoints return { results: [...], total: n }.
    // Photo-list endpoints return [...] directly. Normalize both to the array.
    if let Some(results) = value.get("results").cloned() {
        results
    } else {
        value
    }
}

/// Unsplash image source: persistent headless Chrome (chromiumoxide) solves
/// the Anubis bot-check once, then every tool reads Unsplash's internal
/// `/napi/*` JSON endpoints inside the cleared page context. No API key.
pub struct Unsplash {
    browser: SharedBrowser,
}

impl Unsplash {
    pub fn new(browser: SharedBrowser) -> Self {
        Self { browser }
    }

    pub async fn search_images(
        &self,
        query: &str,
        page: u32,
        per_page: u32,
        orientation: Option<&str>,
        color: Option<&str>,
    ) -> Result<Value> {
        let mut path = format!(
            "/napi/search/photos?query={}&page={}&per_page={}",
            enc(query),
            page,
            per_page.min(30)
        );
        if let Some(o) = orientation {
            path.push_str(&format!("&orientation={}", enc(o)));
        }
        if let Some(c) = color {
            path.push_str(&format!("&color={}", enc(c)));
        }
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn popular_images(&self, page: u32, per_page: u32, order_by: &str) -> Result<Value> {
        let path = format!(
            "/napi/photos?page={}&per_page={}&order_by={}",
            page,
            per_page.min(30),
            enc(order_by)
        );
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn browse_category(&self, category: &str, page: u32, per_page: u32) -> Result<Value> {
        let slug = topic_slug(category);
        let path = format!(
            "/napi/topics/{}/photos?page={}&per_page={}",
            slug,
            page,
            per_page.min(30)
        );
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn user_profile(&self, username: &str, include_photos: bool) -> Result<Value> {
        let path = format!("/napi/users/{}", enc(username));
        let mut profile = self.browser.fetch_napi(&path).await?;
        if include_photos {
            let photos_path = format!(
                "/napi/users/{}/photos?page=1&per_page=12&order_by=latest",
                enc(username)
            );
            match self.browser.fetch_napi(&photos_path).await {
                Ok(photos) => {
                    if let Some(obj) = profile.as_object_mut() {
                        obj.insert("recent_photos".to_string(), as_results(photos));
                    }
                }
                Err(e) => {
                    if let Some(obj) = profile.as_object_mut() {
                        obj.insert(
                            "recent_photos_error".to_string(),
                            Value::String(format!("{e:#}")),
                        );
                    }
                }
            }
        }
        Ok(profile)
    }

    pub async fn image_details(&self, image_id: &str) -> Result<Value> {
        let path = format!("/napi/photos/{}", enc(image_id));
        self.browser.fetch_napi(&path).await
    }

    pub async fn search_by_color(&self, color: &str, page: u32, per_page: u32) -> Result<Value> {
        // napi search requires a query; "photo" keeps it broad while `color` filters.
        let path = format!(
            "/napi/search/photos?query=photo&page={}&per_page={}&color={}",
            page,
            per_page.min(30),
            enc(color)
        );
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn collections(&self, page: u32, per_page: u32, featured: bool) -> Result<Value> {
        let mut path = format!("/napi/collections?page={}&per_page={}", page, per_page.min(30));
        if featured {
            path.push_str("&featured=true");
        }
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn collection_photos(
        &self,
        collection_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Value> {
        let path = format!(
            "/napi/collections/{}/photos?page={}&per_page={}",
            enc(collection_id),
            page,
            per_page.min(30)
        );
        let v = self.browser.fetch_napi(&path).await?;
        Ok(as_results(v))
    }

    pub async fn random_photos(
        &self,
        count: u32,
        query: Option<&str>,
        orientation: Option<&str>,
    ) -> Result<Value> {
        let mut path = format!("/napi/photos/random?count={}", count.min(30));
        if let Some(q) = query {
            if !q.trim().is_empty() {
                path.push_str(&format!("&query={}", enc(q)));
            }
        }
        if let Some(o) = orientation {
            path.push_str(&format!("&orientation={}", enc(o)));
        }
        let v = self.browser.fetch_napi(&path).await?;
        // /random returns an object when count=1, array otherwise.
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::{enc, topic_slug};

    #[test]
    fn encodes_query_values() {
        assert_eq!(enc("mountain landscape"), "mountain%20landscape");
        assert_eq!(enc("black & white"), "black%20%26%20white");
    }

    #[test]
    fn slugifies_categories() {
        assert_eq!(topic_slug("Nature"), "nature");
        assert_eq!(topic_slug("Black & White"), "black-%26-white");
    }
}
