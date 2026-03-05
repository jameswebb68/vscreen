import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import Breadcrumbs from './Breadcrumbs.svelte';
import type { BreadcrumbItem } from '$lib/types/index.js';

describe('Breadcrumbs', () => {
	const items: BreadcrumbItem[] = [
		{ label: 'Home', url: '/' },
		{ label: 'Products', url: '/products' },
		{ label: 'Current Page' }
	];

	it('renders all breadcrumb labels', () => {
		render(Breadcrumbs, { items });
		expect(screen.getByText('Home')).toBeInTheDocument();
		expect(screen.getByText('Products')).toBeInTheDocument();
		expect(screen.getByText('Current Page')).toBeInTheDocument();
	});

	it('links intermediate items with href', () => {
		render(Breadcrumbs, { items });
		const homeLink = screen.getByText('Home').closest('a');
		const productsLink = screen.getByText('Products').closest('a');
		expect(homeLink).toHaveAttribute('href', '/');
		expect(productsLink).toHaveAttribute('href', '/products');
	});

	it('last item is not a link (rendered as span)', () => {
		render(Breadcrumbs, { items });
		const lastItem = screen.getByText('Current Page');
		expect(lastItem.tagName).toBe('SPAN');
		expect(lastItem.closest('a')).toBeNull();
	});

	it('renders separator between items', () => {
		const { container } = render(Breadcrumbs, { items });
		const separators = container.querySelectorAll('li[aria-hidden="true"]');
		expect(separators.length).toBeGreaterThanOrEqual(2);
		expect(separators[0]?.textContent).toBe('/');
	});

	it('handles single item', () => {
		const singleItem: BreadcrumbItem[] = [{ label: 'Only', url: '/only' }];
		render(Breadcrumbs, { items: singleItem });
		expect(screen.getByText('Only')).toBeInTheDocument();
		const lastItem = screen.getByText('Only');
		expect(lastItem.tagName).toBe('SPAN');
	});
});
