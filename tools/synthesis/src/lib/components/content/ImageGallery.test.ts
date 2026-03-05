import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ImageGallery from './ImageGallery.svelte';
import type { ImageItem } from '$lib/types/index.js';

describe('ImageGallery', () => {
	const items: ImageItem[] = [
		{ src: 'https://img.com/1.jpg', alt: 'Photo 1', caption: 'Caption 1' },
		{ src: 'https://img.com/2.jpg', alt: 'Photo 2' },
		{ src: 'https://img.com/3.jpg', caption: 'Caption 3' }
	];

	it('renders all images', () => {
		const { container } = render(ImageGallery, { items });
		const images = container.querySelectorAll('img');
		expect(images).toHaveLength(3);
	});

	it('sets correct src on images', () => {
		const { container } = render(ImageGallery, { items });
		const images = container.querySelectorAll('img');
		expect(images[0]).toHaveAttribute('src', 'https://img.com/1.jpg');
		expect(images[1]).toHaveAttribute('src', 'https://img.com/2.jpg');
	});

	it('sets alt text on images', () => {
		const { container } = render(ImageGallery, { items });
		const images = container.querySelectorAll('img');
		expect(images[0]).toHaveAttribute('alt', 'Photo 1');
		expect(images[1]).toHaveAttribute('alt', 'Photo 2');
	});

	it('falls back to empty alt when not provided', () => {
		const { container } = render(ImageGallery, { items });
		const images = container.querySelectorAll('img');
		expect(images[2]).toHaveAttribute('alt', '');
	});

	it('renders captions where present', () => {
		render(ImageGallery, { items });
		expect(screen.getByText('Caption 1')).toBeInTheDocument();
		expect(screen.getByText('Caption 3')).toBeInTheDocument();
	});

	it('renders title when provided', () => {
		render(ImageGallery, { items, title: 'Gallery' });
		expect(screen.getByText('Gallery')).toBeInTheDocument();
	});

	it('uses figure elements', () => {
		const { container } = render(ImageGallery, { items });
		const figures = container.querySelectorAll('figure');
		expect(figures).toHaveLength(3);
	});

	it('wraps image in link when url is provided', () => {
		const withUrl: ImageItem[] = [
			{ src: 'https://img.com/1.jpg', url: 'https://example.com' }
		];
		const { container } = render(ImageGallery, { items: withUrl });
		const link = container.querySelector('a');
		expect(link).toHaveAttribute('href', 'https://example.com');
		expect(link).toHaveAttribute('target', '_blank');
	});

	it('does not wrap image in link when no url', () => {
		const noUrl: ImageItem[] = [{ src: 'https://img.com/1.jpg' }];
		const { container } = render(ImageGallery, { items: noUrl });
		const link = container.querySelector('a');
		expect(link).not.toBeInTheDocument();
	});

	it('defaults to 3-column grid', () => {
		const { container } = render(ImageGallery, { items });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('lg:grid-cols-3');
	});

	it('applies 2-column grid', () => {
		const { container } = render(ImageGallery, { items, columns: 2 });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('md:grid-cols-2');
	});

	it('applies 4-column grid', () => {
		const { container } = render(ImageGallery, { items, columns: 4 });
		const grid = container.querySelector('.grid');
		expect(grid?.className).toContain('md:grid-cols-4');
	});
});
