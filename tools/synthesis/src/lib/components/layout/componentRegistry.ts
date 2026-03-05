import type {
	Section,
	ArticleItem,
	StatItem,
	ImageItem,
	TableColumn,
	ChartSeries,
	ChartPoint,
	TableRow,
	TimelineEvent,
	BreadcrumbItem,
	KeyValuePair,
	AccordionItem,
	SidebarItem,
	ComparisonColumn,
	ComparisonFeature,
	FilterOption,
	ComponentType
} from '$lib/types/index.js';
import type { Component } from 'svelte';

import CardGrid from '../content/CardGrid.svelte';
import ContentList from '../content/ContentList.svelte';
import ImageGallery from '../content/ImageGallery.svelte';
import StatsRow from '../content/StatsRow.svelte';
import LiveFeed from '../realtime/LiveFeed.svelte';

import DataTable from '../viz/DataTable.svelte';
import BarChart from '../viz/BarChart.svelte';
import LineChart from '../viz/LineChart.svelte';
import PieChart from '../viz/PieChart.svelte';

import Sidebar from '../nav/Sidebar.svelte';
import Breadcrumbs from '../nav/Breadcrumbs.svelte';

import Accordion from '../interactive/Accordion.svelte';
import Timeline from '../interactive/Timeline.svelte';

import KeyValueList from '../blocks/KeyValueList.svelte';
import ComparisonTable from '../composite/ComparisonTable.svelte';

import HeroSectionWrapper from './sections/HeroSectionWrapper.svelte';
import ProgressBarSection from './sections/ProgressBarSection.svelte';
import NotificationBannerSection from './sections/NotificationBannerSection.svelte';
import PaginationSection from './sections/PaginationSection.svelte';
import FilterBarSection from './sections/FilterBarSection.svelte';
import QuoteBlockSection from './sections/QuoteBlockSection.svelte';
import CodeBlockSection from './sections/CodeBlockSection.svelte';
import MarkdownBlockSection from './sections/MarkdownBlockSection.svelte';

function meta(section: Section): Record<string, unknown> {
	return (section.meta ?? {}) as Record<string, unknown>;
}

export interface RegistryEntry {
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	component: Component<any>;
	props: (section: Section) => Record<string, unknown>;
}

export const componentRegistry: Record<string, RegistryEntry> = {
	'card-grid': {
		component: CardGrid,
		props: (s) => ({ items: s.data as ArticleItem[], title: s.title })
	},
	'content-list': {
		component: ContentList,
		props: (s) => ({ items: s.data as ArticleItem[], title: s.title })
	},
	'image-gallery': {
		component: ImageGallery,
		props: (s) => ({ items: s.data as ImageItem[], title: s.title })
	},
	'hero': {
		component: HeroSectionWrapper,
		props: (s) => ({ items: s.data as ArticleItem[] })
	},
	'stats-row': {
		component: StatsRow,
		props: (s) => ({ items: s.data as StatItem[], title: s.title })
	},
	'live-feed': {
		component: LiveFeed,
		props: (s) => ({ items: s.data as ArticleItem[], title: s.title })
	},
	'notification-banner': {
		component: NotificationBannerSection,
		props: (s) => ({ items: s.data })
	},
	'data-table': {
		component: DataTable,
		props: (s) => ({
			columns: (meta(s)['columns'] as TableColumn[]) ?? [],
			rows: s.data as TableRow[],
			title: s.title,
			pageSize: (meta(s)['pageSize'] as number) ?? 20
		})
	},
	'bar-chart': {
		component: BarChart,
		props: (s) => ({
			series: (meta(s)['series'] as ChartSeries[]) ?? [
				{ name: 'Data', data: s.data as ChartPoint[] }
			],
			title: s.title,
			horizontal: (meta(s)['horizontal'] as boolean) ?? false
		})
	},
	'line-chart': {
		component: LineChart,
		props: (s) => ({
			series: (meta(s)['series'] as ChartSeries[]) ?? [
				{ name: 'Data', data: s.data as ChartPoint[] }
			],
			title: s.title,
			xLabel: (meta(s)['xLabel'] as string) ?? undefined,
			yLabel: (meta(s)['yLabel'] as string) ?? undefined
		})
	},
	'pie-chart': {
		component: PieChart,
		props: (s) => ({
			data: s.data as ChartPoint[],
			title: s.title,
			donut: (meta(s)['donut'] as boolean) ?? false
		})
	},
	'progress-bar': {
		component: ProgressBarSection,
		props: (s) => ({ items: s.data, title: s.title })
	},
	'sidebar': {
		component: Sidebar,
		props: (s) => ({
			items: s.data as SidebarItem[],
			activeId: (meta(s)['activeId'] as string) ?? undefined
		})
	},
	'breadcrumbs': {
		component: Breadcrumbs,
		props: (s) => ({ items: s.data as BreadcrumbItem[] })
	},
	'pagination': {
		component: PaginationSection,
		props: (s) => ({
			totalPages: (meta(s)['totalPages'] as number) ?? 1
		})
	},
	'accordion': {
		component: Accordion,
		props: (s) => ({ items: s.data as AccordionItem[], title: s.title })
	},
	'filter-bar': {
		component: FilterBarSection,
		props: (s) => ({ filters: s.data as FilterOption[] })
	},
	'timeline': {
		component: Timeline,
		props: (s) => ({ events: s.data as TimelineEvent[], title: s.title })
	},
	'markdown-block': {
		component: MarkdownBlockSection,
		props: (s) => ({ items: s.data, title: s.title })
	},
	'code-block': {
		component: CodeBlockSection,
		props: (s) => ({ items: s.data, title: s.title })
	},
	'quote-block': {
		component: QuoteBlockSection,
		props: (s) => ({ items: s.data })
	},
	'key-value-list': {
		component: KeyValueList,
		props: (s) => ({ items: s.data as KeyValuePair[], title: s.title })
	},
	'comparison-table': {
		component: ComparisonTable,
		props: (s) => ({
			columns: (meta(s)['columns'] as ComparisonColumn[]) ?? [],
			features: s.data as ComparisonFeature[],
			title: s.title
		})
	}
} satisfies Partial<Record<ComponentType, RegistryEntry>>;
