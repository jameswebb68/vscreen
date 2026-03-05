import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import PageShell from './PageShell.svelte';

const noop = createRawSnippet(() => ({
	render: () => '<span></span>'
}));

describe('PageShell', () => {
	it('renders the title as an h1', () => {
		render(PageShell, { title: 'Dashboard', children: noop });
		const heading = screen.getByRole('heading', { level: 1 });
		expect(heading).toHaveTextContent('Dashboard');
	});

	it('renders the subtitle when provided', () => {
		render(PageShell, { title: 'Page', subtitle: 'Subtitle text', children: noop });
		expect(screen.getByText('Subtitle text')).toBeInTheDocument();
	});

	it('does not render subtitle when absent', () => {
		render(PageShell, { title: 'No Sub', children: noop });
		const subtitle = document.querySelector('p.text-surface-400');
		expect(subtitle).not.toBeInTheDocument();
	});

	it('applies dark theme classes by default', () => {
		const { container } = render(PageShell, { title: 'Dark', children: noop });
		const wrapper = container.firstElementChild;
		expect(wrapper?.className).toContain('dark');
		expect(wrapper?.className).toContain('bg-surface-950');
	});

	it('applies light theme classes', () => {
		const { container } = render(PageShell, {
			title: 'Light',
			theme: 'light',
			children: noop
		});
		const wrapper = container.firstElementChild;
		expect(wrapper?.className).toContain('bg-white');
	});

	it('has a main element', () => {
		render(PageShell, { title: 'Test', children: noop });
		expect(document.querySelector('main')).toBeInTheDocument();
	});

	it('has a header element', () => {
		render(PageShell, { title: 'Test', children: noop });
		expect(document.querySelector('header')).toBeInTheDocument();
	});
});
