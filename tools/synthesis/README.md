# Synthesis Bubble — AI-Driven Frontend Workspace

The Synthesis Bubble is a SvelteKit 5 application that gives AI agents a library of pre-built components to construct custom, interactive web pages from scraped data. It runs as a child process of vscreen and is controlled entirely via MCP tools.

## Setup

```bash
# From the vscreen root directory
cd tools/synthesis
pnpm install
```

That's it. When you run `vscreen --dev --synthesis`, the server starts automatically.

## Manual Development

```bash
# Start the dev server standalone (for component development)
pnpm dev

# Run all quality checks
pnpm validate        # svelte-check + eslint + vitest

# Individual checks
pnpm check           # svelte-check (types + a11y)
pnpm lint            # eslint --max-warnings 0
pnpm test            # vitest run (287 tests)
pnpm test:watch      # vitest in watch mode
```

## Architecture

```
Agent → MCP tools → SvelteKit REST API → In-memory page store → SSE → Browser
```

Pages are JSON documents describing layout + sections + data. The agent never writes Svelte code — it assembles pages from 22 pre-built components by specifying component types and data arrays.

### Data Flow

1. Agent calls `vscreen_synthesis_scrape` to extract structured data from a website
2. Agent calls `vscreen_synthesis_create` with sections referencing component types
3. SvelteKit stores the page in memory and broadcasts via SSE
4. Browser renders the page using `ComponentRenderer` which maps types to Svelte components
5. Agent can push real-time updates via `vscreen_synthesis_push`

## Component Reference

### Content (Phase 1)

| Component | Type Key | Props |
|-----------|----------|-------|
| ArticleCard | — | `title`, `url?`, `image?`, `description?`, `source?` |
| CardGrid | `card-grid` | `items: ArticleItem[]`, `title?` |
| ContentList | `content-list` | `items: ArticleItem[]`, `title?` |
| ImageGallery | `image-gallery` | `items: ImageItem[]`, `title?` |
| HeroSection | `hero` | First item from `ArticleItem[]` (title, description, image) |
| StatsRow | `stats-row` | `items: StatItem[]`, `title?` |
| LiveFeed | `live-feed` | `items: ArticleItem[]`, `title?` |

### Data Visualization (Phase 2)

| Component | Type Key | Data / Meta |
|-----------|----------|-------------|
| DataTable | `data-table` | `data: TableRow[]`, `meta.columns: TableColumn[]`, `meta.pageSize?` |
| BarChart | `bar-chart` | `meta.series: ChartSeries[]`, `meta.horizontal?` |
| LineChart | `line-chart` | `meta.series: ChartSeries[]`, `meta.xLabel?`, `meta.yLabel?` |
| PieChart | `pie-chart` | `data: ChartPoint[]`, `meta.donut?` |
| ProgressBar | `progress-bar` | `data: ProgressData[]` |

### Navigation (Phase 2)

| Component | Type Key | Data |
|-----------|----------|------|
| Sidebar | `sidebar` | `data: SidebarItem[]`, `meta.activeId?` |
| Breadcrumbs | `breadcrumbs` | `data: BreadcrumbItem[]` |
| Pagination | `pagination` | `meta.totalPages` |

### Interactive (Phase 2)

| Component | Type Key | Data |
|-----------|----------|------|
| Accordion | `accordion` | `data: AccordionItem[]`, `title?` |
| FilterBar | `filter-bar` | `data: FilterOption[]` |
| Timeline | `timeline` | `data: TimelineEvent[]`, `title?` |
| Modal | `modal` | Programmatic — used in layouts |

### Content Blocks (Phase 2)

| Component | Type Key | Data |
|-----------|----------|------|
| MarkdownBlock | `markdown-block` | `data: [{content: string}]`, `title?` |
| CodeBlock | `code-block` | `data: [{code: string, language?: string}]`, `title?` |
| QuoteBlock | `quote-block` | `data: [{text, author?, source?}]` |
| KeyValueList | `key-value-list` | `data: KeyValuePair[]`, `title?` |

### Composite (Phase 2)

| Component | Type Key | Data / Meta |
|-----------|----------|-------------|
| ComparisonTable | `comparison-table` | `data: ComparisonFeature[]`, `meta.columns: ComparisonColumn[]` |
| NotificationBanner | `notification-banner` | `data: [{message, type?, dismissible?}]` |

## API Routes

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/pages` | List all pages |
| POST | `/api/pages` | Create a new page |
| GET | `/api/pages/[id]` | Get page by ID |
| PUT | `/api/pages/[id]` | Update a page |
| DELETE | `/api/pages/[id]` | Delete a page |
| POST | `/api/pages/[id]/push` | Push data to a section (triggers SSE) |
| POST | `/api/pages/[id]/save` | Persist page to disk |
| GET | `/api/events` | SSE stream for real-time page updates |

## Page JSON Schema

```typescript
interface SynthesisPage {
  id: string;                              // auto-generated slug from title
  title: string;
  subtitle?: string;
  theme: 'dark' | 'light';
  layout: 'grid' | 'list' | 'split' | 'tabs' | 'freeform';
  sections: Section[];
  createdAt: string;
  updatedAt: string;
}

interface Section {
  id: string;
  component: ComponentType;                // e.g. 'card-grid', 'data-table', 'timeline'
  title?: string;
  data: SectionData;                       // array of items matching the component type
  meta?: Record<string, unknown>;          // component-specific config (columns, series, etc.)
}
```

## Persistence

Pages live in memory by default. Call `POST /api/pages/{id}/save` (or the `vscreen_synthesis_save` MCP tool) to write a page to `tools/synthesis/.data/{id}.json`. On server startup, all persisted pages are loaded automatically.

## Tech Stack

- **SvelteKit 5** with Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`)
- **Tailwind CSS 4** with `@tailwindcss/typography`
- **TypeScript** (strict mode)
- **Vite** with `@vitejs/plugin-basic-ssl` for HTTPS
- **Vitest** + `@testing-library/svelte` for component tests
- **ESLint** (`eslint-plugin-svelte` + `typescript-eslint`, `--max-warnings 0`)
- **pnpm** package manager

## Test Coverage

287 tests across 35 test files covering:
- All 22 components (render, props, edge cases, interactions)
- Server-side page CRUD operations (36 tests)
- SSE broadcast hub (9 tests)
- API route handlers (17 tests)
- File persistence (10 tests)
