import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import ProgressBar from '$lib/components/viz/ProgressBar.svelte';

describe('ProgressBar', () => {
	it('renders label when provided', () => {
		render(ProgressBar, { value: 50, label: 'Progress' });
		expect(screen.getByText('Progress')).toBeInTheDocument();
	});

	it('shows percentage text', () => {
		render(ProgressBar, { value: 75, label: 'Loading' });
		expect(screen.getByText('75%')).toBeInTheDocument();
	});

	it('sets correct width style', () => {
		const { container } = render(ProgressBar, { value: 50 });
		const fill = container.querySelector('.h-full.rounded-full');
		expect(fill).toHaveStyle({ width: '50%' });
	});

	it('uses default max of 100', () => {
		render(ProgressBar, { value: 50, label: 'Half' });
		expect(screen.getByText('50%')).toBeInTheDocument();
	});

	it('clamps to 0% minimum', () => {
		render(ProgressBar, { value: -10, label: 'Negative' });
		expect(screen.getByText('0%')).toBeInTheDocument();
	});

	it('clamps to 100% maximum', () => {
		render(ProgressBar, { value: 150, label: 'Overflow' });
		expect(screen.getByText('100%')).toBeInTheDocument();
	});

	it('handles custom max value', () => {
		render(ProgressBar, { value: 50, max: 200, label: 'Custom' });
		expect(screen.getByText('25%')).toBeInTheDocument();
	});

	it('renders without label', () => {
		const { container } = render(ProgressBar, { value: 60 });
		expect(screen.queryByText('Progress')).not.toBeInTheDocument();
		const barContainer = container.querySelector('.space-y-1');
		expect(barContainer).toBeInTheDocument();
		const fill = container.querySelector('.h-full.rounded-full');
		expect(fill).toHaveStyle({ width: '60%' });
	});
});
