import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import KeyValueList from './KeyValueList.svelte';
import type { KeyValuePair } from '$lib/types/index.js';

describe('KeyValueList', () => {
	const items: KeyValuePair[] = [
		{ key: 'Name', value: 'Alice' },
		{ key: 'Age', value: 30 },
		{ key: 'Website', value: 'example.com', url: 'https://example.com' }
	];

	it('renders title when provided', () => {
		render(KeyValueList, { items, title: 'Details' });
		expect(screen.getByText('Details')).toBeInTheDocument();
	});

	it('renders all key-value pairs', () => {
		render(KeyValueList, { items });
		expect(screen.getByText('Name')).toBeInTheDocument();
		expect(screen.getByText('Alice')).toBeInTheDocument();
		expect(screen.getByText('Age')).toBeInTheDocument();
		expect(screen.getByText('30')).toBeInTheDocument();
		expect(screen.getByText('Website')).toBeInTheDocument();
	});

	it('renders value as link when url provided', () => {
		render(KeyValueList, { items });
		const link = screen.getByRole('link', { name: 'example.com' });
		expect(link).toBeInTheDocument();
		expect(link).toHaveAttribute('href', 'https://example.com');
	});

	it('renders value as text when no url', () => {
		render(KeyValueList, { items });
		expect(screen.getByText('Alice').closest('a')).toBeNull();
		expect(screen.getByText('30').closest('a')).toBeNull();
	});

	it('handles empty items', () => {
		const { container } = render(KeyValueList, { items: [] });
		const dtElements = container.querySelectorAll('dt');
		expect(dtElements).toHaveLength(0);
	});
});
