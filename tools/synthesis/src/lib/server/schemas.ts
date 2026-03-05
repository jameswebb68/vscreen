import { z } from 'zod';

// ---------------------------------------------------------------------------
// Per-component data schemas
// ---------------------------------------------------------------------------

const ArticleItemSchema = z.object({
	title: z.string(),
	url: z.string().optional(),
	image: z.string().optional(),
	description: z.string().optional(),
	source: z.string().optional(),
	sourceColor: z.string().optional(),
	timestamp: z.string().optional()
});

const StatItemSchema = z.object({
	label: z.string(),
	value: z.union([z.string(), z.number()]),
	unit: z.string().optional(),
	trend: z.enum(['up', 'down', 'neutral']).optional()
});

const ImageItemSchema = z.object({
	src: z.string(),
	alt: z.string().optional(),
	caption: z.string().optional(),
	url: z.string().optional()
});

const ChartPointSchema = z.object({
	label: z.string(),
	value: z.number(),
	color: z.string().optional()
});

const TimelineEventSchema = z.object({
	date: z.string(),
	title: z.string(),
	description: z.string().optional(),
	icon: z.string().optional()
});

const BreadcrumbItemSchema = z.object({
	label: z.string(),
	url: z.string().optional()
});

const FilterOptionSchema = z.object({
	id: z.string(),
	label: z.string(),
	active: z.boolean().optional()
});

const KeyValuePairSchema = z.object({
	key: z.string(),
	value: z.union([z.string(), z.number()]),
	url: z.string().optional()
});

const AccordionItemSchema = z.object({
	title: z.string(),
	content: z.string()
});

const SidebarItemSchema: z.ZodType<{
	id: string;
	label: string;
	url?: string;
	icon?: string;
	children?: unknown[];
}> = z.object({
	id: z.string(),
	label: z.string(),
	url: z.string().optional(),
	icon: z.string().optional(),
	children: z.array(z.lazy(() => SidebarItemSchema)).optional()
});

const ComparisonFeatureSchema = z.object({
	label: z.string(),
	values: z.record(z.string(), z.union([z.string(), z.number(), z.boolean()]))
});

const NotificationDataSchema = z.object({
	message: z.string(),
	type: z.enum(['info', 'warning', 'error', 'success']).optional(),
	dismissible: z.boolean().optional()
});

const QuoteDataSchema = z.object({
	text: z.string(),
	author: z.string().optional(),
	source: z.string().optional()
});

const CodeBlockDataSchema = z.object({
	code: z.string(),
	language: z.string().optional()
});

const MarkdownDataSchema = z.object({
	content: z.string()
});

const ProgressDataSchema = z.object({
	value: z.number(),
	max: z.number().optional(),
	label: z.string().optional(),
	color: z.string().optional()
});

const TableRowSchema = z.record(z.string(), z.union([z.string(), z.number(), z.boolean()]));

// ---------------------------------------------------------------------------
// Meta schemas for components that use them
// ---------------------------------------------------------------------------

const TableColumnSchema = z.object({
	key: z.string(),
	label: z.string(),
	sortable: z.boolean().optional(),
	width: z.string().optional()
});

const ChartSeriesSchema = z.object({
	name: z.string(),
	data: z.array(ChartPointSchema),
	color: z.string().optional()
});

const ComparisonColumnSchema = z.object({
	id: z.string(),
	label: z.string(),
	highlight: z.boolean().optional()
});

// ---------------------------------------------------------------------------
// Component type → data schema mapping
// ---------------------------------------------------------------------------

const componentDataSchemas: Record<string, z.ZodType> = {
	'card-grid': z.array(ArticleItemSchema),
	'content-list': z.array(ArticleItemSchema),
	'image-gallery': z.array(ImageItemSchema),
	'hero': z.array(ArticleItemSchema),
	'live-feed': z.array(ArticleItemSchema),
	'stats-row': z.array(StatItemSchema),
	'data-table': z.array(TableRowSchema),
	'bar-chart': z.array(ChartPointSchema),
	'line-chart': z.array(ChartPointSchema),
	'pie-chart': z.array(ChartPointSchema),
	'progress-bar': z.array(ProgressDataSchema),
	'sidebar': z.array(SidebarItemSchema),
	'breadcrumbs': z.array(BreadcrumbItemSchema),
	'pagination': z.array(z.any()),
	'accordion': z.array(AccordionItemSchema),
	'modal': z.array(z.any()),
	'filter-bar': z.array(FilterOptionSchema),
	'timeline': z.array(TimelineEventSchema),
	'markdown-block': z.array(MarkdownDataSchema),
	'code-block': z.array(CodeBlockDataSchema),
	'quote-block': z.array(QuoteDataSchema),
	'key-value-list': z.array(KeyValuePairSchema),
	'comparison-table': z.array(ComparisonFeatureSchema),
	'notification-banner': z.array(NotificationDataSchema)
};

const componentMetaSchemas: Record<string, z.ZodType> = {
	'data-table': z.object({
		columns: z.array(TableColumnSchema),
		pageSize: z.number().optional()
	}).passthrough(),
	'bar-chart': z.object({
		series: z.array(ChartSeriesSchema).optional(),
		horizontal: z.boolean().optional()
	}).passthrough(),
	'line-chart': z.object({
		series: z.array(ChartSeriesSchema).optional(),
		xLabel: z.string().optional(),
		yLabel: z.string().optional()
	}).passthrough(),
	'pie-chart': z.object({
		donut: z.boolean().optional()
	}).passthrough(),
	'pagination': z.object({
		totalPages: z.number()
	}).passthrough(),
	'sidebar': z.object({
		activeId: z.string().optional()
	}).passthrough(),
	'comparison-table': z.object({
		columns: z.array(ComparisonColumnSchema)
	}).passthrough()
};

// ---------------------------------------------------------------------------
// Component types enum
// ---------------------------------------------------------------------------

const COMPONENT_TYPES = [
	'card-grid', 'content-list', 'image-gallery', 'hero', 'live-feed', 'stats-row',
	'data-table', 'bar-chart', 'line-chart', 'pie-chart', 'progress-bar',
	'sidebar', 'breadcrumbs', 'pagination',
	'accordion', 'modal', 'filter-bar', 'timeline',
	'markdown-block', 'code-block', 'quote-block', 'key-value-list',
	'comparison-table', 'notification-banner'
] as const;

// ---------------------------------------------------------------------------
// Component aliases — maps intuitive names to canonical component types
// ---------------------------------------------------------------------------

const COMPONENT_ALIASES: Record<string, string> = {
	'article-list': 'card-grid',
	articles: 'card-grid',
	'news-grid': 'card-grid',
	'article-grid': 'card-grid',
	'news-list': 'content-list',
	feed: 'live-feed',
	chart: 'bar-chart',
	table: 'data-table',
	kv: 'key-value-list',
	kvlist: 'key-value-list',
	stats: 'stats-row',
	images: 'image-gallery',
	gallery: 'image-gallery',
	markdown: 'markdown-block',
	code: 'code-block',
	quote: 'quote-block',
	notification: 'notification-banner',
	alert: 'notification-banner',
	compare: 'comparison-table',
	progress: 'progress-bar'
};

export function resolveComponentAlias(component: string): string {
	return COMPONENT_ALIASES[component] ?? component;
}

export function resolveAliasesInSections(sections: unknown[]): void {
	for (const s of sections) {
		if (s && typeof s === 'object' && 'component' in s) {
			const sec = s as Record<string, unknown>;
			if (typeof sec.component === 'string') {
				sec.component = resolveComponentAlias(sec.component);
			}
		}
	}
}

// ---------------------------------------------------------------------------
// Section schema
// ---------------------------------------------------------------------------

const BaseSectionSchema = z.object({
	id: z.string(),
	component: z.enum(COMPONENT_TYPES),
	title: z.string().optional(),
	data: z.array(z.any()),
	meta: z.record(z.string(), z.unknown()).optional()
});

// ---------------------------------------------------------------------------
// Public validation functions
// ---------------------------------------------------------------------------

export interface ValidationError {
	sectionId: string;
	component: string;
	field: 'data' | 'meta';
	message: string;
}

export function validateSection(section: unknown): ValidationError[] {
	const errors: ValidationError[] = [];

	const baseResult = BaseSectionSchema.safeParse(section);
	if (!baseResult.success) {
		const s = section as Record<string, unknown>;
		errors.push({
			sectionId: (s?.id as string) ?? '(unknown)',
			component: (s?.component as string) ?? '(unknown)',
			field: 'data',
			message: baseResult.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`).join('; ')
		});
		return errors;
	}

	const sec = baseResult.data;

	const dataSchema = componentDataSchemas[sec.component];
	if (dataSchema) {
		const dataResult = dataSchema.safeParse(sec.data);
		if (!dataResult.success) {
			errors.push({
				sectionId: sec.id,
				component: sec.component,
				field: 'data',
				message: dataResult.error.issues
					.map((i) => {
						const path = i.path.length > 0 ? `data[${i.path.join('.')}]` : 'data';
						return `${path}: ${i.message}`;
					})
					.join('; ')
			});
		}
	}

	if (sec.meta) {
		const metaSchema = componentMetaSchemas[sec.component];
		if (metaSchema) {
			const metaResult = metaSchema.safeParse(sec.meta);
			if (!metaResult.success) {
				errors.push({
					sectionId: sec.id,
					component: sec.component,
					field: 'meta',
					message: metaResult.error.issues
						.map((i) => {
							const path = i.path.length > 0 ? `meta.${i.path.join('.')}` : 'meta';
							return `${path}: ${i.message}`;
						})
						.join('; ')
				});
			}
		}
	}

	return errors;
}

export function validateSections(sections: unknown[]): ValidationError[] {
	return sections.flatMap((s) => validateSection(s));
}

export const CreatePageSchema = z.object({
	title: z.string().min(1, 'title is required'),
	subtitle: z.string().optional(),
	theme: z.enum(['dark', 'light']).optional(),
	accentColor: z.string().optional(),
	layout: z.enum(['grid', 'list', 'split', 'tabs', 'freeform']).optional(),
	sections: z.array(z.any()).optional()
});

export function formatValidationErrors(errors: ValidationError[]): string {
	return errors
		.map((e) => `Section "${e.sectionId}" (${e.component}): ${e.field} — ${e.message}`)
		.join('\n');
}
