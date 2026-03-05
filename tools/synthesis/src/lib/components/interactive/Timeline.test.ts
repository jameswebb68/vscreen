import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import Timeline from './Timeline.svelte';
import type { TimelineEvent } from '$lib/types/index.js';

describe('Timeline', () => {
	const events: TimelineEvent[] = [
		{ date: '2024-01-01', title: 'Event 1', description: 'Description 1' },
		{ date: '2024-02-01', title: 'Event 2' },
		{ date: '2024-03-01', title: 'Event 3', description: 'Description 3', icon: '★' }
	];

	it('renders title when provided', () => {
		render(Timeline, { props: { events, title: 'Timeline Title' } });
		expect(screen.getByText('Timeline Title')).toBeInTheDocument();
	});

	it('renders all event titles', () => {
		render(Timeline, { props: { events } });
		expect(screen.getByText('Event 1')).toBeInTheDocument();
		expect(screen.getByText('Event 2')).toBeInTheDocument();
		expect(screen.getByText('Event 3')).toBeInTheDocument();
	});

	it('shows event dates', () => {
		render(Timeline, { props: { events } });
		expect(screen.getByText('2024-01-01')).toBeInTheDocument();
		expect(screen.getByText('2024-02-01')).toBeInTheDocument();
		expect(screen.getByText('2024-03-01')).toBeInTheDocument();
	});

	it('shows event descriptions when provided', () => {
		render(Timeline, { props: { events } });
		expect(screen.getByText('Description 1')).toBeInTheDocument();
		expect(screen.getByText('Description 3')).toBeInTheDocument();
	});

	it('handles empty events', () => {
		const { container } = render(Timeline, { props: { events: [] } });
		const timeElements = container.querySelectorAll('time');
		expect(timeElements).toHaveLength(0);
	});
});
