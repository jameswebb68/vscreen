import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ContentList from './ContentList.svelte';
import type { ArticleItem } from '$lib/types/index.js';

describe('ContentList', () => {
	const items: ArticleItem[] = [
		{ title: 'Story 1', url: 'https://a.com', description: 'Desc 1', source: 'CNN' },
		{ title: 'Story 2', url: 'https://b.com', source: 'BBC', timestamp: '10m ago' }
	];

	it('renders all article items', () => {
		render(ContentList, { items });
		expect(screen.getByText('Story 1')).toBeInTheDocument();
		expect(screen.getByText('Story 2')).toBeInTheDocument();
	});

	it('renders the title when provided', () => {
		render(ContentList, { items, title: 'Headlines' });
		expect(screen.getByText('Headlines')).toBeInTheDocument();
	});

	it('does not render title when absent', () => {
		render(ContentList, { items });
		const heading = document.querySelector('h2');
		expect(heading).not.toBeInTheDocument();
	});

	it('renders descriptions where present', () => {
		render(ContentList, { items });
		expect(screen.getByText('Desc 1')).toBeInTheDocument();
	});

	it('renders source badges', () => {
		render(ContentList, { items });
		expect(screen.getByText('CNN')).toBeInTheDocument();
		expect(screen.getByText('BBC')).toBeInTheDocument();
	});

	it('renders timestamp when present', () => {
		render(ContentList, { items });
		expect(screen.getByText('10m ago')).toBeInTheDocument();
	});

	it('renders as list items', () => {
		const { container } = render(ContentList, { items });
		const listItems = container.querySelectorAll('li');
		expect(listItems).toHaveLength(2);
	});

	it('renders images where present', () => {
		const withImage: ArticleItem[] = [
			{ title: 'With Img', url: 'https://x.com', image: 'https://img.com/1.jpg' }
		];
		const { container } = render(ContentList, { items: withImage });
		const img = container.querySelector('img');
		expect(img).toBeInTheDocument();
		expect(img).toHaveAttribute('src', 'https://img.com/1.jpg');
	});

	it('handles empty items list', () => {
		const { container } = render(ContentList, { items: [] });
		const list = container.querySelector('ul');
		expect(list).toBeInTheDocument();
		expect(list?.children).toHaveLength(0);
	});
});
