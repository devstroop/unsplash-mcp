# Unsplash MCP

MCP server for Unsplash image discovery via web scraping. No API key needed.

## Setup

```bash
npm install
npm run build
```

Add to your MCP config:
```json
{
  "unsplash-mcp": {
    "command": "node",
    "args": ["path/to/unsplash-mcp/dist/index.js"]
  }
}
```

## Tools

| Tool | Description |
|------|-------------|
| `search_images` | Search by keywords, orientation, color |
| `get_popular_images` | Trending/popular images |
| `browse_category` | Browse by category |
| `get_user_profile` | Photographer info and portfolio |
| `get_image_details` | Full metadata for a specific image |
| `search_by_color` | Find images by dominant color |
| `get_random_photos` | Random image selection |

## License

MIT
