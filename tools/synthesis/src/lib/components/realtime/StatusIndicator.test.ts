import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import StatusIndicator from './StatusIndicator.svelte';

describe('StatusIndicator', () => {
	it('renders connected status', () => {
		render(StatusIndicator, { status: 'connected' });
		expect(screen.getByText('connected')).toBeInTheDocument();
	});

	it('renders disconnected status', () => {
		render(StatusIndicator, { status: 'disconnected' });
		expect(screen.getByText('disconnected')).toBeInTheDocument();
	});

	it('renders loading status', () => {
		render(StatusIndicator, { status: 'loading' });
		expect(screen.getByText('loading')).toBeInTheDocument();
	});

	it('uses custom label when provided', () => {
		render(StatusIndicator, { status: 'connected', label: 'Live' });
		expect(screen.getByText('Live')).toBeInTheDocument();
		expect(screen.queryByText('connected')).not.toBeInTheDocument();
	});

	it('applies green dot for connected', () => {
		const { container } = render(StatusIndicator, { status: 'connected' });
		const dot = container.querySelector('.bg-green-400');
		expect(dot).toBeInTheDocument();
	});

	it('applies red dot for disconnected', () => {
		const { container } = render(StatusIndicator, { status: 'disconnected' });
		const dot = container.querySelector('.bg-red-400');
		expect(dot).toBeInTheDocument();
	});

	it('applies pulsing yellow dot for loading', () => {
		const { container } = render(StatusIndicator, { status: 'loading' });
		const dot = container.querySelector('.bg-yellow-400');
		expect(dot).toBeInTheDocument();
		expect(dot?.className).toContain('animate-pulse');
	});

	it('renders the dot as a small circle', () => {
		const { container } = render(StatusIndicator, { status: 'connected' });
		const dot = container.querySelector('.rounded-full');
		expect(dot).toBeInTheDocument();
		expect(dot?.className).toContain('h-2');
		expect(dot?.className).toContain('w-2');
	});
});
