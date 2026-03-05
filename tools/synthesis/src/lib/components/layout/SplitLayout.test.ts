import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import SplitLayout from './SplitLayout.svelte';

const noop = createRawSnippet(() => ({
	render: () => '<span></span>'
}));

describe('SplitLayout', () => {
	it('renders a grid container', () => {
		const { container } = render(SplitLayout, { left: noop, right: noop });
		const grid = container.querySelector('.grid');
		expect(grid).toBeInTheDocument();
	});

	it('uses 1fr 1fr ratio by default', () => {
		const { container } = render(SplitLayout, { left: noop, right: noop });
		const grid = container.querySelector('.grid');
		expect(grid?.getAttribute('style')).toContain('1fr 1fr');
	});

	it('applies custom ratio', () => {
		const { container } = render(SplitLayout, {
			left: noop,
			right: noop,
			ratio: '2fr 1fr'
		});
		const grid = container.querySelector('.grid');
		expect(grid?.getAttribute('style')).toContain('2fr 1fr');
	});

	it('renders two child divs', () => {
		const { container } = render(SplitLayout, { left: noop, right: noop });
		const grid = container.querySelector('.grid');
		expect(grid?.children).toHaveLength(2);
	});
});
