import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import SourceBadge from './SourceBadge.svelte';

describe('SourceBadge', () => {
	it('renders the source text', () => {
		render(SourceBadge, { source: 'CNN' });
		expect(screen.getByText('CNN')).toBeInTheDocument();
	});

	it('applies the default color for known sources', () => {
		const { container } = render(SourceBadge, { source: 'CNN' });
		const badge = container.querySelector('span');
		expect(badge?.className).toContain('bg-red-600');
	});

	it('applies the default color for BBC', () => {
		const { container } = render(SourceBadge, { source: 'BBC' });
		const badge = container.querySelector('span');
		expect(badge?.className).toContain('bg-amber-600');
	});

	it('falls back to indigo for unknown sources', () => {
		const { container } = render(SourceBadge, { source: 'Unknown' });
		const badge = container.querySelector('span');
		expect(badge?.className).toContain('bg-indigo-600');
	});

	it('uses custom color when provided', () => {
		const { container } = render(SourceBadge, { source: 'CNN', color: 'bg-purple-500' });
		const badge = container.querySelector('span');
		expect(badge?.className).toContain('bg-purple-500');
		expect(badge?.className).not.toContain('bg-red-600');
	});

	it('is case-insensitive for default color lookup', () => {
		const { container } = render(SourceBadge, { source: 'cnn' });
		const badge = container.querySelector('span');
		expect(badge?.className).toContain('bg-red-600');
	});

	it('renders as a span element', () => {
		const { container } = render(SourceBadge, { source: 'Test' });
		const badge = container.querySelector('span');
		expect(badge).toBeInTheDocument();
		expect(badge?.tagName).toBe('SPAN');
	});
});
