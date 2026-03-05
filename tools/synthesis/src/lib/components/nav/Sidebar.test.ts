import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import Sidebar from './Sidebar.svelte';
import type { SidebarItem } from '$lib/types/index.js';

describe('Sidebar', () => {
	const items: SidebarItem[] = [
		{ id: 'a', label: 'Item A', url: '/a' },
		{ id: 'b', label: 'Item B', url: '/b' },
		{ id: 'c', label: 'Item C', url: '/c' }
	];

	it('renders all top-level items', () => {
		render(Sidebar, { items });
		expect(screen.getByText('Item A')).toBeInTheDocument();
		expect(screen.getByText('Item B')).toBeInTheDocument();
		expect(screen.getByText('Item C')).toBeInTheDocument();
	});

	it('renders item labels', () => {
		render(Sidebar, { items });
		expect(screen.getByText('Item A')).toBeInTheDocument();
		expect(screen.getByText('Item B')).toBeInTheDocument();
	});

	it('active item gets accent styling class', () => {
		render(Sidebar, { items, activeId: 'b' });
		const activeLink = screen.getByText('Item B').closest('a');
		expect(activeLink?.className).toContain('text-accent-400');
		expect(activeLink?.className).toContain('bg-accent-500/20');
	});

	it('renders children when present', () => {
		const itemsWithChildren: SidebarItem[] = [
			{
				id: 'parent',
				label: 'Parent',
				url: '/parent',
				children: [
					{ id: 'child1', label: 'Child 1', url: '/child1' },
					{ id: 'child2', label: 'Child 2', url: '/child2' }
				]
			}
		];
		render(Sidebar, { items: itemsWithChildren });
		expect(screen.getByText('Parent')).toBeInTheDocument();
		expect(screen.getByText('Child 1')).toBeInTheDocument();
		expect(screen.getByText('Child 2')).toBeInTheDocument();
	});

	it('section headers (items without URL) render as span not link', () => {
		const itemsWithSection: SidebarItem[] = [
			{ id: 'section', label: 'Section Header' },
			{ id: 'link', label: 'Link Item', url: '/link' }
		];
		render(Sidebar, { items: itemsWithSection });
		const sectionEl = screen.getByText('Section Header');
		expect(sectionEl.tagName).toBe('SPAN');
		expect(screen.getByText('Link Item').closest('a')).toBeInTheDocument();
	});

	it('renders icons when provided', () => {
		const itemsWithIcons: SidebarItem[] = [
			{ id: 'icon', label: 'With Icon', url: '/icon', icon: '📁' }
		];
		render(Sidebar, { items: itemsWithIcons });
		expect(screen.getByText('📁')).toBeInTheDocument();
		expect(screen.getByText('With Icon')).toBeInTheDocument();
	});

	it('handles empty items', () => {
		const { container } = render(Sidebar, { items: [] });
		const links = container.querySelectorAll('a');
		const spans = container.querySelectorAll('span.block');
		expect(links).toHaveLength(0);
		expect(spans).toHaveLength(0);
	});
});
