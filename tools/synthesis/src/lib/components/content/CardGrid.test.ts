import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import CardGrid from './CardGrid.svelte';
import type { ArticleItem } from '$lib/types/index.js';

describe('CardGrid', () => {
	const items: ArticleItem[] = [
		{ title: 'Article A', url: 'https://a.com' },
		{ title: 'Article B', url: 'https://b.com' },
		{ title: 'Article C', url: 'https://c.com' }
	];

	it('renders all article items', () => {
		render(CardGrid, { items });
		expect(screen.getByText('Article A')).toBeInTheDocument();
		expect(screen.getByText('Article B')).toBeInTheDocument();
		expect(screen.getByText('Article C')).toBeInTheDocument();
	});

	it('renders the title when provided', () => {
		render(CardGrid, { items, title: 'News Grid' });
		expect(screen.getByText('News Grid')).toBeInTheDocument();
	});

	it('does not render title when absent', () => {
		render(CardGrid, { items });
		expect(screen.queryByText('News Grid')).not.toBeInTheDocument();
	});

	it('renders correct number of links', () => {
		render(CardGrid, { items });
		const links = screen.getAllByRole('link');
		expect(links).toHaveLength(3);
	});

	it('applies 4-column grid by default', () => {
		const { container } = render(CardGrid, { items });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('lg:grid-cols-4');
	});

	it('applies 2-column grid when columns=2', () => {
		const { container } = render(CardGrid, { items, columns: 2 });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('md:grid-cols-2');
		expect(grid?.className).not.toContain('lg:grid-cols-4');
	});

	it('applies 3-column grid when columns=3', () => {
		const { container } = render(CardGrid, { items, columns: 3 });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('lg:grid-cols-3');
	});

	it('renders empty grid gracefully', () => {
		const { container } = render(CardGrid, { items: [] });
		const grid = container.querySelector('.grid');
		expect(grid).toBeInTheDocument();
		expect(grid?.children).toHaveLength(0);
	});
});
