import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import LineChart from '$lib/components/viz/LineChart.svelte';
import type { ChartSeries } from '$lib/types/index.js';

describe('LineChart', () => {
	const singleSeries: ChartSeries[] = [
		{
			name: 'Revenue',
			data: [
				{ label: 'Jan', value: 100 },
				{ label: 'Feb', value: 150 },
				{ label: 'Mar', value: 120 }
			]
		}
	];

	const multiSeries: ChartSeries[] = [
		{
			name: 'Product A',
			data: [
				{ label: 'Q1', value: 50 },
				{ label: 'Q2', value: 75 }
			]
		},
		{
			name: 'Product B',
			data: [
				{ label: 'Q1', value: 30 },
				{ label: 'Q2', value: 60 }
			]
		}
	];

	it('renders title when provided', () => {
		render(LineChart, { series: singleSeries, title: 'Revenue Over Time' });
		expect(screen.getByText('Revenue Over Time')).toBeInTheDocument();
	});

	it('renders SVG element', () => {
		const { container } = render(LineChart, { series: singleSeries });
		const svg = container.querySelector('svg');
		expect(svg).toBeInTheDocument();
	});

	it('renders path elements for each series', () => {
		const { container } = render(LineChart, { series: multiSeries });
		const paths = container.querySelectorAll('path');
		expect(paths.length).toBeGreaterThanOrEqual(2);
	});

	it('renders x-axis labels', () => {
		render(LineChart, { series: singleSeries });
		expect(screen.getByText('Jan')).toBeInTheDocument();
		expect(screen.getByText('Feb')).toBeInTheDocument();
		expect(screen.getByText('Mar')).toBeInTheDocument();
	});

	it('shows legend when multiple series', () => {
		render(LineChart, { series: multiSeries });
		expect(screen.getByText('Product A')).toBeInTheDocument();
		expect(screen.getByText('Product B')).toBeInTheDocument();
	});

	it('handles empty series', () => {
		const { container } = render(LineChart, { series: [] });
		const chartContainer = container.querySelector('.rounded-lg');
		expect(chartContainer).toBeInTheDocument();
	});
});
