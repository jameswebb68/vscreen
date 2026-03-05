import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import LiveFeed from './LiveFeed.svelte';
import type { ArticleItem } from '$lib/types/index.js';

describe('LiveFeed', () => {
	const items: ArticleItem[] = [
		{ title: 'Live Story 1', url: 'https://a.com', source: 'CNN' },
		{ title: 'Live Story 2', url: 'https://b.com', source: 'BBC' }
	];

	it('renders all items', () => {
		render(LiveFeed, { items });
		expect(screen.getByText('Live Story 1')).toBeInTheDocument();
		expect(screen.getByText('Live Story 2')).toBeInTheDocument();
	});

	it('renders the title when provided', () => {
		render(LiveFeed, { items, title: 'Breaking News' });
		expect(screen.getByText('Breaking News')).toBeInTheDocument();
	});

	it('shows connected status by default', () => {
		render(LiveFeed, { items });
		expect(screen.getByText('Live')).toBeInTheDocument();
	});

	it('shows disconnected status when connected=false', () => {
		render(LiveFeed, { items, connected: false });
		expect(screen.getByText('Offline')).toBeInTheDocument();
	});

	it('renders connected indicator dot as green', () => {
		const { container } = render(LiveFeed, { items });
		const dot = container.querySelector('.bg-green-400');
		expect(dot).toBeInTheDocument();
	});

	it('renders disconnected indicator dot as red', () => {
		const { container } = render(LiveFeed, { items, connected: false });
		const dot = container.querySelector('.bg-red-400');
		expect(dot).toBeInTheDocument();
	});

	it('delegates article rendering to ContentList', () => {
		const { container } = render(LiveFeed, { items });
		const listItems = container.querySelectorAll('li');
		expect(listItems).toHaveLength(2);
	});

	it('handles empty items', () => {
		const { container } = render(LiveFeed, { items: [] });
		const listItems = container.querySelectorAll('li');
		expect(listItems).toHaveLength(0);
	});
});
