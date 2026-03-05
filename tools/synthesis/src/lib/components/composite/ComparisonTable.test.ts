import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ComparisonTable from './ComparisonTable.svelte';
import type { ComparisonColumn, ComparisonFeature } from '$lib/types/index.js';

describe('ComparisonTable', () => {
	const columns: ComparisonColumn[] = [
		{ id: 'basic', label: 'Basic', highlight: false },
		{ id: 'pro', label: 'Pro', highlight: true }
	];

	const features: ComparisonFeature[] = [
		{ label: 'Feature A', values: { basic: true, pro: true } },
		{ label: 'Feature B', values: { basic: false, pro: true } },
		{ label: 'Feature C', values: { basic: '5', pro: 'Unlimited' } }
	];

	it('renders title when provided', () => {
		render(ComparisonTable, {
			columns,
			features,
			title: 'Plan Comparison'
		});
		expect(screen.getByText('Plan Comparison')).toBeInTheDocument();
	});

	it('renders column headers', () => {
		render(ComparisonTable, { columns, features });
		expect(screen.getByText('Feature')).toBeInTheDocument();
		expect(screen.getByText('Basic')).toBeInTheDocument();
		expect(screen.getByText('Pro')).toBeInTheDocument();
	});

	it('renders feature labels', () => {
		render(ComparisonTable, { columns, features });
		expect(screen.getByText('Feature A')).toBeInTheDocument();
		expect(screen.getByText('Feature B')).toBeInTheDocument();
		expect(screen.getByText('Feature C')).toBeInTheDocument();
	});

	it('shows checkmark for true values', () => {
		render(ComparisonTable, { columns, features });
		const checkmarks = screen.getAllByText('✓');
		expect(checkmarks.length).toBeGreaterThanOrEqual(2);
	});

	it('shows X for false values', () => {
		render(ComparisonTable, { columns, features });
		const crosses = screen.getAllByText('✗');
		expect(crosses.length).toBeGreaterThanOrEqual(1);
	});

	it('highlights column when highlight is true', () => {
		const { container } = render(ComparisonTable, { columns, features });
		const headers = container.querySelectorAll('th');
		const proHeader = Array.from(headers).find((th) => th.textContent === 'Pro');
		expect(proHeader?.className).toContain('bg-accent-500/10');
		expect(proHeader?.className).toContain('text-accent-400');
	});
});
