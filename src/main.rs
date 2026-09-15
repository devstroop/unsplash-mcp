mod browser;
mod unsplash;

use std::sync::Arc;

use browser::{BrowserManager, SharedBrowser};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    schemars,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use tracing::error;
use unsplash::Unsplash;

// ---------- tool argument schemas ----------

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SearchImagesArgs {
    /// Search keywords (e.g. "mountain landscape", "city night")
    pub query: String,
    /// Page number for pagination
    #[serde(default = "default_page")]
    pub page: u32,
    /// Number of results per page
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// Image orientation filter
    pub orientation: Option<String>,
    /// Filter by dominant color
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct PopularImagesArgs {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// Sort order: latest, oldest, popular
    #[serde(default = "default_order")]
    pub order_by: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct BrowseCategoryArgs {
    /// Category or topic name (e.g. "nature", "architecture", "food")
    pub category: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UserProfileArgs {
    /// Unsplash username
    pub username: String,
    /// Include user's recent photos
    #[serde(default = "default_true")]
    pub include_photos: bool,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ImageDetailsArgs {
    /// Unsplash image ID
    pub image_id: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SearchByColorArgs {
    /// Color name (black_and_white, black, white, yellow, orange, red, purple, magenta, green, teal, blue)
    pub color: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CollectionsArgs {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    #[serde(default)]
    pub featured: bool,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CollectionPhotosArgs {
    /// Collection ID
    pub collection_id: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RandomPhotosArgs {
    /// Number of random photos to get
    #[serde(default = "default_count")]
    pub count: u32,
    /// Optional query to filter random photos
    pub query: Option<String>,
    /// Image orientation filter
    pub orientation: Option<String>,
}

fn default_page() -> u32 {
    1
}
fn default_per_page() -> u32 {
    20
}
fn default_count() -> u32 {
    10
}
fn default_order() -> String {
    "popular".to_string()
}
fn default_true() -> bool {
    true
}

// ---------- server ----------

#[derive(Clone)]
pub struct UnsplashMcp {
    tool_router: ToolRouter<Self>,
    browser: SharedBrowser,
}

#[tool_router(router = tool_router)]
impl UnsplashMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            browser: Arc::new(BrowserManager::new()),
        }
    }

    fn api(&self) -> Unsplash {
        Unsplash::new(self.browser.clone())
    }

    fn ok(value: serde_json::Value, title: String) -> Result<CallToolResult, ErrorData> {
        let pretty = serde_json::to_string_pretty(&value)
            .unwrap_or_else(|_| value.to_string());
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "# {title}\n\n{pretty}"
        ))]))
    }

    fn err(e: anyhow::Error) -> ErrorData {
        error!(error = ?e, "tool failed");
        ErrorData::internal_error(format!("{e:#}"), None)
    }

    #[tool(description = "Search for images on Unsplash using keywords and filters")]
    async fn search_images(
        &self,
        Parameters(args): Parameters<SearchImagesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query is required".to_string(), None));
        }
        let q = args.query.clone();
        self.api()
            .search_images(
                &q,
                args.page.max(1),
                args.per_page.clamp(1, 30),
                args.orientation.as_deref(),
                args.color.as_deref(),
            )
            .await
            .map(|v| Self::ok(v, format!("Search Results for \"{q}\"")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get trending and popular images from Unsplash")]
    async fn get_popular_images(
        &self,
        Parameters(args): Parameters<PopularImagesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.api()
            .popular_images(args.page.max(1), args.per_page.clamp(1, 30), &args.order_by)
            .await
            .map(|v| Self::ok(v, "Popular Images".to_string()).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Browse images by specific categories or topics")]
    async fn browse_category(
        &self,
        Parameters(args): Parameters<BrowseCategoryArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.category.trim().is_empty() {
            return Err(ErrorData::invalid_params("category is required".to_string(), None));
        }
        let c = args.category.clone();
        self.api()
            .browse_category(&c, args.page.max(1), args.per_page.clamp(1, 30))
            .await
            .map(|v| Self::ok(v, format!("Category: {c}")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get photographer information and their portfolio")]
    async fn get_user_profile(
        &self,
        Parameters(args): Parameters<UserProfileArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.username.trim().is_empty() {
            return Err(ErrorData::invalid_params("username is required".to_string(), None));
        }
        let u = args.username.clone();
        self.api()
            .user_profile(&u, args.include_photos)
            .await
            .map(|v| Self::ok(v, format!("User Profile: {u}")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get comprehensive information about a specific image")]
    async fn get_image_details(
        &self,
        Parameters(args): Parameters<ImageDetailsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.image_id.trim().is_empty() {
            return Err(ErrorData::invalid_params("image_id is required".to_string(), None));
        }
        let id = args.image_id.clone();
        self.api()
            .image_details(&id)
            .await
            .map(|v| Self::ok(v, format!("Image Details: {id}")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Find images with specific dominant colors")]
    async fn search_by_color(
        &self,
        Parameters(args): Parameters<SearchByColorArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.color.trim().is_empty() {
            return Err(ErrorData::invalid_params("color is required".to_string(), None));
        }
        let c = args.color.clone();
        self.api()
            .search_by_color(&c, args.page.max(1), args.per_page.clamp(1, 30))
            .await
            .map(|v| Self::ok(v, format!("Images with color: {c}")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get curated collections of images")]
    async fn get_collections(
        &self,
        Parameters(args): Parameters<CollectionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.api()
            .collections(args.page.max(1), args.per_page.clamp(1, 30), args.featured)
            .await
            .map(|v| Self::ok(v, "Collections".to_string()).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get photos from a specific collection")]
    async fn get_collection_photos(
        &self,
        Parameters(args): Parameters<CollectionPhotosArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.collection_id.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "collection_id is required".to_string(),
                None,
            ));
        }
        let id = args.collection_id.clone();
        self.api()
            .collection_photos(&id, args.page.max(1), args.per_page.clamp(1, 30))
            .await
            .map(|v| Self::ok(v, format!("Collection Photos: {id}")).unwrap())
            .map_err(Self::err)
    }

    #[tool(description = "Get random high-quality photos")]
    async fn get_random_photos(
        &self,
        Parameters(args): Parameters<RandomPhotosArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.api()
            .random_photos(
                args.count.clamp(1, 30),
                args.query.as_deref(),
                args.orientation.as_deref(),
            )
            .await
            .map(|v| Self::ok(v, "Random Photos".to_string()).unwrap())
            .map_err(Self::err)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for UnsplashMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new("unsplash-mcp", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Unsplash image discovery. The server fully manages its own browser: \
             hardened stealth headless Chrome first, automatic fallback to a \
             server-managed headed Chrome if denied. No setup needed — just call \
             the tools."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // CRITICAL for stdio MCP: logs must go to stderr, never stdout.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let service = UnsplashMcp::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
