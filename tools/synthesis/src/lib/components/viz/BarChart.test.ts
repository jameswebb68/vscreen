import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import BarChart from '$lib/components/viz/BarChart.svelte';
import type { ChartSeries } from '$lib/types/index.js';

describe('BarChart', () => {
	const singleSeries: ChartSeries[] = [
		{
			name: 'Sales',
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
		render(BarChart, { series: singleSeries, title: 'Monthly Sales' });
		expect(screen.getByText('Monthly Sales')).toBeInTheDocument();
	});

	it('renders SVG element for vertical chart', () => {
		const { container } = render(BarChart, { series: singleSeries });
		const svg = container.querySelector('svg');
		expect(svg).toBeInTheDocument();
	});

	it('renders horizontal bars when horizontal prop is true', () => {
		const { container } = render(BarChart, { series: singleSeries, horizontal: true });
		const svg = container.querySelector('svg');
		expect(svg).not.toBeInTheDocument();
		const horizontalDiv = container.querySelector('.space-y-2');
		expect(horizontalDiv).toBeInTheDocument();
	});

	it('shows legend when multiple series', () => {
		render(BarChart, { series: multiSeries });
		expect(screen.getByText('Product A')).toBeInTheDocument();
		expect(screen.getByText('Product B')).toBeInTheDocument();
	});

	it('handles single series without legend', () => {
		const { container } = render(BarChart, { series: singleSeries });
		const legendContainer = container.querySelector('.mt-3.flex.flex-wrap');
		expect(legendContainer).not.toBeInTheDocument();
	});

	it('renders correct number of labels', () => {
		render(BarChart, { series: singleSeries });
		expect(screen.getByText('Jan')).toBeInTheDocument();
		expect(screen.getByText('Feb')).toBeInTheDocument();
		expect(screen.getByText('Mar')).toBeInTheDocument();
	});

	it('handles empty data', () => {
		const { container } = render(BarChart, { series: [] });
		const chartContainer = container.querySelector('.rounded-lg');
		expect(chartContainer).toBeInTheDocument();
	});
});
