import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import FilterBar from './FilterBar.svelte';
import type { FilterOption } from '$lib/types/index.js';

describe('FilterBar', () => {
	const filters: FilterOption[] = [
		{ id: 'a', label: 'Filter A', active: true },
		{ id: 'b', label: 'Filter B', active: false },
		{ id: 'c', label: 'Filter C', active: false }
	];

	it('renders all filter labels', () => {
		render(FilterBar, { filters, onToggle: () => {} });
		expect(screen.getByText('Filter A')).toBeInTheDocument();
		expect(screen.getByText('Filter B')).toBeInTheDocument();
		expect(screen.getByText('Filter C')).toBeInTheDocument();
	});

	it('active filters have accent styling', () => {
		render(FilterBar, { filters, onToggle: () => {} });
		const activeButton = screen.getByText('Filter A').closest('button');
		expect(activeButton?.className).toContain('text-accent-400');
		expect(activeButton?.className).toContain('bg-accent-500/20');
	});

	it('inactive filters have default styling', () => {
		render(FilterBar, { filters, onToggle: () => {} });
		const inactiveButton = screen.getByText('Filter B').closest('button');
		expect(inactiveButton?.className).toContain('bg-surface-800');
		expect(inactiveButton?.className).toContain('text-surface-400');
	});

	it('calls onToggle with filter id on click', async () => {
		let toggledId: string | null = null;
		const handleToggle = (id: string) => {
			toggledId = id;
		};
		render(FilterBar, { filters, onToggle: handleToggle });
		await fireEvent.click(screen.getByText('Filter B'));
		expect(toggledId).toBe('b');
	});

	it('handles empty filters', () => {
		const { container } = render(FilterBar, { filters: [], onToggle: () => {} });
		const buttons = container.querySelectorAll('button');
		expect(buttons).toHaveLength(0);
	});
});
