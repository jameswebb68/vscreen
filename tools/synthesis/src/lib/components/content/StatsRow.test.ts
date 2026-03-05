import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import StatsRow from './StatsRow.svelte';
import type { StatItem } from '$lib/types/index.js';

describe('StatsRow', () => {
	const items: StatItem[] = [
		{ label: 'Users', value: 1500, trend: 'up' },
		{ label: 'Revenue', value: '$25k', unit: 'USD', trend: 'down' },
		{ label: 'Sessions', value: 3200, trend: 'neutral' },
		{ label: 'Bounce', value: '42%' }
	];

	it('renders all stat labels', () => {
		render(StatsRow, { items });
		expect(screen.getByText('Users')).toBeInTheDocument();
		expect(screen.getByText('Revenue')).toBeInTheDocument();
		expect(screen.getByText('Sessions')).toBeInTheDocument();
		expect(screen.getByText('Bounce')).toBeInTheDocument();
	});

	it('renders numeric values', () => {
		render(StatsRow, { items });
		expect(screen.getByText('1500')).toBeInTheDocument();
		expect(screen.getByText('3200')).toBeInTheDocument();
	});

	it('renders string values', () => {
		render(StatsRow, { items });
		expect(screen.getByText('$25k')).toBeInTheDocument();
		expect(screen.getByText('42%')).toBeInTheDocument();
	});

	it('renders unit when provided', () => {
		render(StatsRow, { items });
		expect(screen.getByText('USD')).toBeInTheDocument();
	});

	it('renders up trend arrow', () => {
		render(StatsRow, { items });
		expect(screen.getByText('↑')).toBeInTheDocument();
	});

	it('renders down trend arrow', () => {
		render(StatsRow, { items });
		expect(screen.getByText('↓')).toBeInTheDocument();
	});

	it('renders neutral trend indicator', () => {
		render(StatsRow, { items });
		expect(screen.getByText('—')).toBeInTheDocument();
	});

	it('applies correct trend colors', () => {
		const { container } = render(StatsRow, { items });
		const trendSpans = container.querySelectorAll('.text-green-400, .text-red-400, .text-surface-500');
		expect(trendSpans.length).toBeGreaterThanOrEqual(3);
	});

	it('renders title when provided', () => {
		render(StatsRow, { items, title: 'Key Metrics' });
		expect(screen.getByText('Key Metrics')).toBeInTheDocument();
	});

	it('does not render title when absent', () => {
		render(StatsRow, { items });
		const heading = document.querySelector('h2');
		expect(heading).not.toBeInTheDocument();
	});

	it('handles empty items', () => {
		const { container } = render(StatsRow, { items: [] });
		const grid = container.querySelector('.grid');
		expect(grid).toBeInTheDocument();
		expect(grid?.children).toHaveLength(0);
	});
});
