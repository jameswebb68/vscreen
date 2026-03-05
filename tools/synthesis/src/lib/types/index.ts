// ---------------------------------------------------------------------------
// Component and layout types
// ---------------------------------------------------------------------------

export type ComponentType =
	// Phase 1: Content
	| 'card-grid'
	| 'content-list'
	| 'image-gallery'
	| 'hero'
	| 'live-feed'
	| 'stats-row'
	// Phase 2: Data Visualization
	| 'data-table'
	| 'bar-chart'
	| 'line-chart'
	| 'pie-chart'
	| 'progress-bar'
	// Phase 2: Navigation
	| 'sidebar'
	| 'breadcrumbs'
	| 'pagination'
	// Phase 2: Interactive
	| 'accordion'
	| 'modal'
	| 'filter-bar'
	| 'timeline'
	// Phase 2: Content Blocks
	| 'markdown-block'
	| 'code-block'
	| 'quote-block'
	| 'key-value-list'
	// Phase 2: Composite
	| 'comparison-table'
	| 'notification-banner';

export type LayoutType = 'grid' | 'list' | 'split' | 'tabs' | 'freeform';

export type ThemeType = 'dark' | 'light';

// ---------------------------------------------------------------------------
// Phase 1 data items
// ---------------------------------------------------------------------------

export interface ArticleItem {
	title: string;
	url?: string;
	image?: string;
	description?: string;
	source?: string;
	sourceColor?: string;
	timestamp?: string;
}

export interface StatItem {
	label: string;
	value: string | number;
	unit?: string;
	trend?: 'up' | 'down' | 'neutral';
}

export interface ImageItem {
	src: string;
	alt?: string;
	caption?: string;
	url?: string;
}

// ---------------------------------------------------------------------------
// Phase 2 data items
// ---------------------------------------------------------------------------

export interface TableColumn {
	key: string;
	label: string;
	sortable?: boolean;
	width?: string;
}

export interface TableRow {
	[key: string]: string | number | boolean;
}

export interface TableData {
	columns: TableColumn[];
	rows: TableRow[];
}

export interface ChartPoint {
	label: string;
	value: number;
	color?: string;
}

export interface ChartSeries {
	name: string;
	data: ChartPoint[];
	color?: string;
}

export interface ChartData {
	series: ChartSeries[];
	xLabel?: string;
	yLabel?: string;
}

export interface TimelineEvent {
	date: string;
	title: string;
	description?: string;
	icon?: string;
}

export interface BreadcrumbItem {
	label: string;
	url?: string;
}

export interface FilterOption {
	id: string;
	label: string;
	active?: boolean;
}

export interface KeyValuePair {
	key: string;
	value: string | number;
	url?: string;
}

export interface AccordionItem {
	title: string;
	content: string;
}

export interface SidebarItem {
	id: string;
	label: string;
	url?: string;
	icon?: string;
	children?: SidebarItem[];
}

export interface ComparisonColumn {
	id: string;
	label: string;
	highlight?: boolean;
}

export interface ComparisonFeature {
	label: string;
	values: Record<string, string | number | boolean>;
}

export type NotificationType = 'info' | 'warning' | 'error' | 'success';

export interface NotificationData {
	message: string;
	type: NotificationType;
	dismissible?: boolean;
}

export interface QuoteData {
	text: string;
	author?: string;
	source?: string;
}

export interface CodeBlockData {
	code: string;
	language?: string;
}

export interface MarkdownData {
	content: string;
}

export interface ProgressData {
	value: number;
	max?: number;
	label?: string;
	color?: string;
}

// ---------------------------------------------------------------------------
// Section data union
// ---------------------------------------------------------------------------

export type SectionData =
	| ArticleItem[]
	| StatItem[]
	| ImageItem[]
	| TableRow[]
	| ChartPoint[]
	| TimelineEvent[]
	| BreadcrumbItem[]
	| FilterOption[]
	| KeyValuePair[]
	| AccordionItem[]
	| SidebarItem[]
	| QuoteData[]
	| CodeBlockData[]
	| MarkdownData[]
	| ProgressData[]
	| NotificationData[]
	| ComparisonFeature[];

// ---------------------------------------------------------------------------
// Section and page structures
// ---------------------------------------------------------------------------

export interface Section {
	id: string;
	component: ComponentType;
	title?: string;
	data: SectionData;
	meta?: Record<string, unknown>;
}

export interface SynthesisPage {
	id: string;
	title: string;
	subtitle?: string;
	theme: ThemeType;
	accentColor?: string;
	layout: LayoutType;
	sections: Section[];
	createdAt: string;
	updatedAt: string;
}

// ---------------------------------------------------------------------------
// API request types
// ---------------------------------------------------------------------------

export interface CreatePageRequest {
	title: string;
	subtitle?: string;
	theme?: ThemeType;
	accentColor?: string;
	layout?: LayoutType;
	sections?: Section[];
}

export interface PushDataRequest {
	section_id: string;
	data: SectionData;
}

export interface UpdatePageRequest {
	title?: string;
	subtitle?: string;
	theme?: ThemeType;
	accentColor?: string;
	layout?: LayoutType;
	sections?: Section[];
}
