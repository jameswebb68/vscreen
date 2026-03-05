import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import Accordion from './Accordion.svelte';
import type { AccordionItem } from '$lib/types/index.js';

describe('Accordion', () => {
	const items: AccordionItem[] = [
		{ title: 'Item 1', content: 'Content 1' },
		{ title: 'Item 2', content: 'Content 2' },
		{ title: 'Item 3', content: 'Content 3' }
	];

	it('renders title when provided', () => {
		render(Accordion, { items, title: 'Accordion Title' });
		expect(screen.getByText('Accordion Title')).toBeInTheDocument();
	});

	it('renders all item titles', () => {
		render(Accordion, { items });
		expect(screen.getByText('Item 1')).toBeInTheDocument();
		expect(screen.getByText('Item 2')).toBeInTheDocument();
		expect(screen.getByText('Item 3')).toBeInTheDocument();
	});

	it('content hidden by default', () => {
		render(Accordion, { items });
		expect(screen.queryByText('Content 1')).not.toBeInTheDocument();
		expect(screen.queryByText('Content 2')).not.toBeInTheDocument();
	});

	it('clicking item shows its content', async () => {
		render(Accordion, { items });
		const item1Button = screen.getByText('Item 1');
		await fireEvent.click(item1Button);
		expect(screen.getByText('Content 1')).toBeInTheDocument();
	});

	it('clicking open item hides it', async () => {
		render(Accordion, { items });
		const item1Button = screen.getByText('Item 1');
		await fireEvent.click(item1Button);
		expect(screen.getByText('Content 1')).toBeInTheDocument();
		await fireEvent.click(item1Button);
		expect(screen.queryByText('Content 1')).not.toBeInTheDocument();
	});

	it('handles empty items', () => {
		const { container } = render(Accordion, { items: [] });
		const buttons = container.querySelectorAll('button');
		expect(buttons).toHaveLength(0);
	});
});
