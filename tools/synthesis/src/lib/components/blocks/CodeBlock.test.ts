import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import CodeBlock from './CodeBlock.svelte';

describe('CodeBlock', () => {
	it('renders code content', () => {
		render(CodeBlock, { code: 'const x = 1;' });
		expect(screen.getByText('const x = 1;')).toBeInTheDocument();
	});

	it('shows language label when provided', () => {
		render(CodeBlock, { code: 'fn main() {}', language: 'rust' });
		expect(screen.getByText('rust')).toBeInTheDocument();
	});

	it('shows title when provided', () => {
		render(CodeBlock, { code: 'x', title: 'Example.ts' });
		expect(screen.getByText('Example.ts')).toBeInTheDocument();
	});

	it('has copy button', () => {
		render(CodeBlock, { code: 'hello' });
		expect(screen.getByRole('button', { name: /copy/i })).toBeInTheDocument();
	});

	it('renders code in pre/code elements', () => {
		const { container } = render(CodeBlock, { code: 'const a = 1;' });
		const pre = container.querySelector('pre');
		const code = container.querySelector('code');
		expect(pre).toBeInTheDocument();
		expect(code).toBeInTheDocument();
		expect(code?.textContent).toBe('const a = 1;');
	});
});
