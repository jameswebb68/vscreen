import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ComponentRenderer from './ComponentRenderer.svelte';
import type {
	Section,
	ArticleItem,
	StatItem,
	ImageItem,
	ChartPoint,
	TableRow,
	TimelineEvent,
	AccordionItem,
	KeyValuePair,
	ComparisonFeature,
	BreadcrumbItem,
	FilterOption
} from '$lib/types/index.js';

// ---------------------------------------------------------------------------
// Phase 1: Content components
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Content', () => {
	it('renders card-grid', () => {
		const section: Section = {
			id: 'cg',
			component: 'card-grid',
			title: 'Card Grid',
			data: [
				{ title: 'Article 1', url: 'https://a.com' },
				{ title: 'Article 2', url: 'https://b.com' }
			] as ArticleItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Card Grid')).toBeInTheDocument();
		expect(screen.getByText('Article 1')).toBeInTheDocument();
		expect(screen.getByText('Article 2')).toBeInTheDocument();
	});

	it('renders content-list', () => {
		const section: Section = {
			id: 'cl',
			component: 'content-list',
			title: 'Content List',
			data: [{ title: 'Item 1', url: 'https://a.com' }] as ArticleItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Content List')).toBeInTheDocument();
		expect(screen.getByText('Item 1')).toBeInTheDocument();
	});

	it('renders image-gallery', () => {
		const section: Section = {
			id: 'ig',
			component: 'image-gallery',
			title: 'Gallery',
			data: [
				{ src: 'https://img.com/1.jpg', alt: 'Photo 1' },
				{ src: 'https://img.com/2.jpg', alt: 'Photo 2' }
			] as ImageItem[]
		};
		const { container } = render(ComponentRenderer, { section });
		expect(screen.getByText('Gallery')).toBeInTheDocument();
		expect(container.querySelectorAll('img')).toHaveLength(2);
	});

	it('renders hero', () => {
		const section: Section = {
			id: 'hero',
			component: 'hero',
			data: [{ title: 'Hero Title', description: 'Hero desc' }] as ArticleItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Hero Title')).toBeInTheDocument();
		expect(screen.getByText('Hero desc')).toBeInTheDocument();
	});

	it('handles hero with empty data', () => {
		const section: Section = {
			id: 'empty-hero',
			component: 'hero',
			data: [] as ArticleItem[]
		};
		const { container } = render(ComponentRenderer, { section });
		expect(container.querySelector('section')).not.toBeInTheDocument();
	});

	it('renders stats-row', () => {
		const section: Section = {
			id: 'stats',
			component: 'stats-row',
			title: 'Stats',
			data: [{ label: 'Users', value: 1500, trend: 'up' }] as StatItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Stats')).toBeInTheDocument();
		expect(screen.getByText('Users')).toBeInTheDocument();
		expect(screen.getByText('1500')).toBeInTheDocument();
	});

	it('renders live-feed', () => {
		const section: Section = {
			id: 'feed',
			component: 'live-feed',
			title: 'Live Feed',
			data: [{ title: 'Live Item', url: 'https://a.com' }] as ArticleItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Live Feed')).toBeInTheDocument();
		expect(screen.getByText('Live Item')).toBeInTheDocument();
	});
});

// ---------------------------------------------------------------------------
// Phase 2: Visualization
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Visualization', () => {
	it('renders bar-chart', () => {
		const section: Section = {
			id: 'bar',
			component: 'bar-chart',
			title: 'Revenue',
			data: [
				{ label: 'Jan', value: 42 },
				{ label: 'Feb', value: 53 }
			] as ChartPoint[]
		};
		const { container } = render(ComponentRenderer, { section });
		expect(screen.getByText('Revenue')).toBeInTheDocument();
		expect(screen.getByText('Jan')).toBeInTheDocument();
		expect(screen.getByText('Feb')).toBeInTheDocument();
		expect(container.querySelectorAll('rect').length).toBeGreaterThan(0);
	});

	it('renders line-chart', () => {
		const section: Section = {
			id: 'line',
			component: 'line-chart',
			title: 'DAU',
			data: [
				{ label: 'Mon', value: 100 },
				{ label: 'Tue', value: 200 }
			] as ChartPoint[]
		};
		const { container } = render(ComponentRenderer, { section });
		expect(screen.getByText('DAU')).toBeInTheDocument();
		expect(container.querySelector('svg')).toBeInTheDocument();
	});

	it('renders pie-chart', () => {
		const section: Section = {
			id: 'pie',
			component: 'pie-chart',
			title: 'Traffic',
			data: [
				{ label: 'Search', value: 60 },
				{ label: 'Direct', value: 40 }
			] as ChartPoint[]
		};
		const { container } = render(ComponentRenderer, { section });
		expect(screen.getByText('Traffic')).toBeInTheDocument();
		expect(container.querySelector('svg')).toBeInTheDocument();
		expect(screen.getByText('Search')).toBeInTheDocument();
	});

	it('renders progress-bar', () => {
		const section: Section = {
			id: 'prog',
			component: 'progress-bar',
			title: 'Goals',
			data: [
				{ value: 75, max: 100, label: 'Revenue' },
				{ value: 50, max: 100, label: 'Signups' }
			]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Goals')).toBeInTheDocument();
		expect(screen.getByText('Revenue')).toBeInTheDocument();
		expect(screen.getByText('Signups')).toBeInTheDocument();
	});

	it('renders data-table', () => {
		const section: Section = {
			id: 'dt',
			component: 'data-table',
			title: 'Pages',
			data: [
				{ page: '/home', views: '1000' },
				{ page: '/about', views: '500' }
			] as TableRow[],
			meta: {
				columns: [
					{ key: 'page', label: 'Page' },
					{ key: 'views', label: 'Views' }
				]
			}
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Pages')).toBeInTheDocument();
		expect(screen.getByText('/home')).toBeInTheDocument();
		expect(screen.getByText('1000')).toBeInTheDocument();
	});
});

// ---------------------------------------------------------------------------
// Phase 2: Interactive
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Interactive', () => {
	it('renders accordion', () => {
		const section: Section = {
			id: 'acc',
			component: 'accordion',
			title: 'FAQ',
			data: [
				{ title: 'Question 1', content: 'Answer 1' },
				{ title: 'Question 2', content: 'Answer 2' }
			] as AccordionItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('FAQ')).toBeInTheDocument();
		expect(screen.getByText('Question 1')).toBeInTheDocument();
		expect(screen.getByText('Question 2')).toBeInTheDocument();
	});

	it('renders timeline', () => {
		const section: Section = {
			id: 'tl',
			component: 'timeline',
			title: 'Milestones',
			data: [
				{ date: '2026-01', title: 'Alpha', description: 'Launch' },
				{ date: '2026-03', title: 'GA', description: 'Release' }
			] as TimelineEvent[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Milestones')).toBeInTheDocument();
		expect(screen.getByText('Alpha')).toBeInTheDocument();
		expect(screen.getByText('GA')).toBeInTheDocument();
	});

	it('renders filter-bar', () => {
		const section: Section = {
			id: 'fb',
			component: 'filter-bar',
			data: [
				{ id: 'f1', label: 'Active', active: true },
				{ id: 'f2', label: 'Inactive', active: false }
			] as FilterOption[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Active')).toBeInTheDocument();
		expect(screen.getByText('Inactive')).toBeInTheDocument();
	});
});

// ---------------------------------------------------------------------------
// Phase 2: Content Blocks
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Content Blocks', () => {
	it('renders quote-block', () => {
		const section: Section = {
			id: 'qt',
			component: 'quote-block',
			data: [{ text: 'Great product.', author: 'Jane', source: 'Corp' }]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText(/Great product/)).toBeInTheDocument();
		expect(screen.getByText(/Jane/)).toBeInTheDocument();
	});

	it('handles quote-block with empty data', () => {
		const section: Section = {
			id: 'qt-empty',
			component: 'quote-block',
			data: []
		};
		const { container } = render(ComponentRenderer, { section });
		expect(container.querySelector('blockquote')).not.toBeInTheDocument();
	});

	it('renders key-value-list', () => {
		const section: Section = {
			id: 'kv',
			component: 'key-value-list',
			title: 'Status',
			data: [
				{ key: 'Uptime', value: '99.9%' },
				{ key: 'Latency', value: '42ms' }
			] as KeyValuePair[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Status')).toBeInTheDocument();
		expect(screen.getByText('Uptime')).toBeInTheDocument();
		expect(screen.getByText('99.9%')).toBeInTheDocument();
	});

	it('renders code-block', () => {
		const section: Section = {
			id: 'cb',
			component: 'code-block',
			title: 'Example',
			data: [{ code: 'console.log("hi")', language: 'javascript' }]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Example')).toBeInTheDocument();
		expect(screen.getByText('console.log("hi")')).toBeInTheDocument();
	});

	it('handles code-block with empty data', () => {
		const section: Section = {
			id: 'cb-empty',
			component: 'code-block',
			data: []
		};
		const { container } = render(ComponentRenderer, { section });
		expect(container.querySelector('pre')).not.toBeInTheDocument();
	});

	it('renders markdown-block', () => {
		const section: Section = {
			id: 'md',
			component: 'markdown-block',
			title: 'Docs',
			data: [{ content: '**bold** text' }]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Docs')).toBeInTheDocument();
	});
});

// ---------------------------------------------------------------------------
// Phase 2: Navigation
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Navigation', () => {
	it('renders breadcrumbs', () => {
		const section: Section = {
			id: 'bc',
			component: 'breadcrumbs',
			data: [
				{ label: 'Home', url: '/' },
				{ label: 'Docs', url: '/docs' },
				{ label: 'API' }
			] as BreadcrumbItem[]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Home')).toBeInTheDocument();
		expect(screen.getByText('Docs')).toBeInTheDocument();
		expect(screen.getByText('API')).toBeInTheDocument();
	});

	it('renders pagination', () => {
		const section: Section = {
			id: 'pg',
			component: 'pagination',
			data: [],
			meta: { totalPages: 5 }
		};
		const { container } = render(ComponentRenderer, { section });
		expect(container.querySelectorAll('button').length).toBeGreaterThan(0);
	});
});

// ---------------------------------------------------------------------------
// Phase 2: Composite
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Composite', () => {
	it('renders notification-banner', () => {
		const section: Section = {
			id: 'notif',
			component: 'notification-banner',
			data: [
				{ message: 'Maintenance tonight', type: 'warning', dismissible: true },
				{ message: 'New feature released', type: 'info' }
			]
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Maintenance tonight')).toBeInTheDocument();
		expect(screen.getByText('New feature released')).toBeInTheDocument();
	});

	it('renders comparison-table', () => {
		const section: Section = {
			id: 'comp',
			component: 'comparison-table',
			title: 'Plans',
			data: [
				{ label: 'Storage', values: { free: '1GB', pro: '100GB' } },
				{ label: 'Support', values: { free: 'Email', pro: 'Priority' } }
			] as ComparisonFeature[],
			meta: {
				columns: [
					{ id: 'free', label: 'Free' },
					{ id: 'pro', label: 'Pro' }
				]
			}
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Plans')).toBeInTheDocument();
		expect(screen.getByText('Storage')).toBeInTheDocument();
		expect(screen.getByText('1GB')).toBeInTheDocument();
		expect(screen.getByText('Free')).toBeInTheDocument();
		expect(screen.getByText('Pro')).toBeInTheDocument();
	});
});

// ---------------------------------------------------------------------------
// Fallback
// ---------------------------------------------------------------------------

describe('ComponentRenderer — Fallback', () => {
	it('renders fallback for unknown component', () => {
		const section: Section = {
			id: 'unknown',
			component: 'widget-x' as never,
			data: []
		};
		render(ComponentRenderer, { section });
		expect(screen.getByText('Unknown component: widget-x')).toBeInTheDocument();
	});

	it('renders with empty data arrays without crashing', () => {
		const types = [
			'card-grid',
			'content-list',
			'stats-row',
			'live-feed',
			'bar-chart',
			'line-chart',
			'progress-bar',
			'accordion',
			'timeline',
			'key-value-list',
			'notification-banner',
			'breadcrumbs',
			'filter-bar'
		] as const;

		for (const component of types) {
			const section: Section = { id: `empty-${component}`, component, data: [] };
			expect(() => render(ComponentRenderer, { section })).not.toThrow();
		}
	});
});
