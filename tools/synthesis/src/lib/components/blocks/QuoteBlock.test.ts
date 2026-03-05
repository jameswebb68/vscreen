import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import QuoteBlock from './QuoteBlock.svelte';

describe('QuoteBlock', () => {
	it('renders quote text', () => {
		render(QuoteBlock, { text: 'To be or not to be' });
		expect(screen.getByText(/To be or not to be/)).toBeInTheDocument();
	});

	it('shows author when provided', () => {
		render(QuoteBlock, { text: 'Quote', author: 'Shakespeare' });
		expect(screen.getByText(/Shakespeare/)).toBeInTheDocument();
	});

	it('shows source when provided', () => {
		render(QuoteBlock, { text: 'Quote', source: 'Hamlet' });
		expect(screen.getByText(/\(Hamlet\)/)).toBeInTheDocument();
	});

	it('renders as blockquote element', () => {
		const { container } = render(QuoteBlock, { text: 'Quote' });
		const blockquote = container.querySelector('blockquote');
		expect(blockquote).toBeInTheDocument();
		expect(blockquote?.textContent).toContain('Quote');
	});

	it('works without author and source', () => {
		render(QuoteBlock, { text: 'Simple quote' });
		expect(screen.getByText(/Simple quote/)).toBeInTheDocument();
	});
});
