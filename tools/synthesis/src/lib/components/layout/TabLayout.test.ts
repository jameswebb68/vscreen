import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import TabLayout from './TabLayout.svelte';

const noop = createRawSnippet(() => ({
	render: () => '<span></span>'
}));

describe('TabLayout', () => {
	const tabs = [
		{ id: 'tab1', label: 'Tab One' },
		{ id: 'tab2', label: 'Tab Two' },
		{ id: 'tab3', label: 'Tab Three' }
	];

	it('renders all tab labels', () => {
		render(TabLayout, { tabs, children: noop });
		expect(screen.getByText('Tab One')).toBeInTheDocument();
		expect(screen.getByText('Tab Two')).toBeInTheDocument();
		expect(screen.getByText('Tab Three')).toBeInTheDocument();
	});

	it('renders tabs as buttons', () => {
		render(TabLayout, { tabs, children: noop });
		const buttons = screen.getAllByRole('button');
		expect(buttons).toHaveLength(3);
	});

	it('first tab is active by default', async () => {
		const { container } = render(TabLayout, { tabs, children: noop });
		await new Promise((r) => setTimeout(r, 10));
		const buttons = container.querySelectorAll('button');
		expect(buttons[0]?.className).toContain('border-accent-500');
	});

	it('switches active tab on click', async () => {
		const { container } = render(TabLayout, { tabs, children: noop });
		await new Promise((r) => setTimeout(r, 10));

		const secondTab = screen.getByText('Tab Two');
		await fireEvent.click(secondTab);

		const buttons = container.querySelectorAll('button');
		expect(buttons[1]?.className).toContain('border-accent-500');
		expect(buttons[0]?.className).not.toContain('border-accent-500');
	});

	it('handles empty tabs array', () => {
		const { container } = render(TabLayout, { tabs: [], children: noop });
		const buttons = container.querySelectorAll('button');
		expect(buttons).toHaveLength(0);
	});
});
