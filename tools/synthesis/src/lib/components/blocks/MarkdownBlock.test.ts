import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import MarkdownBlock from './MarkdownBlock.svelte';

describe('MarkdownBlock', () => {
	it('renders title when provided', () => {
		render(MarkdownBlock, { content: 'Hello', title: 'Block Title' });
		expect(screen.getByText('Block Title')).toBeInTheDocument();
	});

	it('renders bold text as <strong>', () => {
		const { container } = render(MarkdownBlock, {
			content: 'This is **bold** text'
		});
		const strong = container.querySelector('strong');
		expect(strong).toBeInTheDocument();
		expect(strong?.textContent).toBe('bold');
	});

	it('renders italic text as <em>', () => {
		const { container } = render(MarkdownBlock, {
			content: 'This is *italic* text'
		});
		const em = container.querySelector('em');
		expect(em).toBeInTheDocument();
		expect(em?.textContent).toBe('italic');
	});

	it('renders inline code as <code>', () => {
		const { container } = render(MarkdownBlock, {
			content: 'Use the `code` function'
		});
		const code = container.querySelector('code');
		expect(code).toBeInTheDocument();
		expect(code?.textContent).toBe('code');
	});

	it('renders headings', () => {
		const { container } = render(MarkdownBlock, {
			content: '# H1\n## H2\n### H3'
		});
		expect(container.querySelector('h1')?.textContent).toBe('H1');
		expect(container.querySelector('h2')?.textContent).toBe('H2');
		expect(container.querySelector('h3')?.textContent).toBe('H3');
	});
});
