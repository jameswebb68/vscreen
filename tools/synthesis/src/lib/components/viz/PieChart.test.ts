import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import PieChart from '$lib/components/viz/PieChart.svelte';
import type { ChartPoint } from '$lib/types/index.js';

describe('PieChart', () => {
	const data: ChartPoint[] = [
		{ label: 'A', value: 50 },
		{ label: 'B', value: 30 },
		{ label: 'C', value: 20 }
	];

	const singleItem: ChartPoint[] = [{ label: 'Only', value: 100 }];

	it('renders title when provided', () => {
		render(PieChart, { data, title: 'Distribution' });
		expect(screen.getByText('Distribution')).toBeInTheDocument();
	});

	it('renders SVG element', () => {
		const { container } = render(PieChart, { data });
		const svg = container.querySelector('svg');
		expect(svg).toBeInTheDocument();
	});

	it('renders path for each data point', () => {
		const { container } = render(PieChart, { data });
		const paths = container.querySelectorAll('svg path');
		expect(paths.length).toBe(3);
	});

	it('shows percentage labels in legend', () => {
		render(PieChart, { data });
		expect(screen.getByText('50%')).toBeInTheDocument();
		expect(screen.getByText('30%')).toBeInTheDocument();
		expect(screen.getByText('20%')).toBeInTheDocument();
	});

	it('handles single item data', () => {
		const { container } = render(PieChart, { data: singleItem });
		expect(screen.getByText('Only')).toBeInTheDocument();
		expect(screen.getByText('100%')).toBeInTheDocument();
		const paths = container.querySelectorAll('svg path');
		expect(paths.length).toBe(1);
	});

	it('handles empty data', () => {
		const { container } = render(PieChart, { data: [] });
		const chartContainer = container.querySelector('.rounded-lg');
		expect(chartContainer).toBeInTheDocument();
		const paths = container.querySelectorAll('svg path');
		expect(paths.length).toBe(0);
	});
});
