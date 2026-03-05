import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ArticleCard from './ArticleCard.svelte';
import type { ArticleItem } from '$lib/types/index.js';

describe('ArticleCard', () => {
	const baseItem: ArticleItem = {
		title: 'Test Article',
		url: 'https://example.com/article',
		description: 'A test description',
		source: 'TestSource',
		timestamp: '2025-01-15T10:00:00Z'
	};

	it('renders the article title', () => {
		render(ArticleCard, { item: baseItem });
		expect(screen.getByText('Test Article')).toBeInTheDocument();
	});

	it('renders a link with the correct URL', () => {
		render(ArticleCard, { item: baseItem });
		const link = screen.getByRole('link');
		expect(link).toHaveAttribute('href', 'https://example.com/article');
	});

	it('opens link in new tab', () => {
		render(ArticleCard, { item: baseItem });
		const link = screen.getByRole('link');
		expect(link).toHaveAttribute('target', '_blank');
		expect(link).toHaveAttribute('rel', 'noopener noreferrer');
	});

	it('renders the description', () => {
		render(ArticleCard, { item: baseItem });
		expect(screen.getByText('A test description')).toBeInTheDocument();
	});

	it('renders the source badge', () => {
		render(ArticleCard, { item: baseItem });
		expect(screen.getByText('TestSource')).toBeInTheDocument();
	});

	it('renders the timestamp', () => {
		render(ArticleCard, { item: baseItem });
		expect(screen.getByText('2025-01-15T10:00:00Z')).toBeInTheDocument();
	});

	it('renders image when provided', () => {
		const item: ArticleItem = { ...baseItem, image: 'https://example.com/img.jpg' };
		const { container } = render(ArticleCard, { item });
		const img = container.querySelector('img');
		expect(img).toBeInTheDocument();
		expect(img).toHaveAttribute('src', 'https://example.com/img.jpg');
	});

	it('shows "No image" placeholder when image is absent', () => {
		const item: ArticleItem = { title: 'No Image', url: 'https://example.com' };
		render(ArticleCard, { item });
		expect(screen.getByText('No image')).toBeInTheDocument();
	});

	it('falls back to # when url is absent', () => {
		const item: ArticleItem = { title: 'No URL' };
		render(ArticleCard, { item });
		const link = screen.getByRole('link');
		expect(link).toHaveAttribute('href', '#');
	});

	it('hides description when not provided', () => {
		const item: ArticleItem = { title: 'Minimal' };
		render(ArticleCard, { item });
		expect(screen.queryByText('A test description')).not.toBeInTheDocument();
	});

	it('hides source badge when source is not provided', () => {
		const item: ArticleItem = { title: 'No Source' };
		render(ArticleCard, { item });
		expect(screen.queryByText('TestSource')).not.toBeInTheDocument();
	});

	it('hides timestamp when not provided', () => {
		const item: ArticleItem = { title: 'No Timestamp' };
		render(ArticleCard, { item });
		const timeEl = document.querySelector('time');
		expect(timeEl).not.toBeInTheDocument();
	});
});
