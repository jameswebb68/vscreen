import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Modal from './Modal.svelte';

const childrenSnippet = createRawSnippet(() => ({
	render: () => '<span>Modal content</span>'
}));

describe('Modal', () => {
	const onClose = () => {
		/* noop */
	};

	it('renders when open is true', () => {
		render(Modal, {
			open: true,
			onClose,
			children: childrenSnippet
		});
		expect(screen.getByText('Modal content')).toBeInTheDocument();
	});

	it('does not render when open is false', () => {
		render(Modal, {
			open: false,
			onClose,
			children: childrenSnippet
		});
		expect(screen.queryByText('Modal content')).not.toBeInTheDocument();
	});

	it('shows title when provided', () => {
		render(Modal, {
			open: true,
			title: 'Modal Title',
			onClose,
			children: childrenSnippet
		});
		expect(screen.getByText('Modal Title')).toBeInTheDocument();
	});

	it('calls onClose on close button click', async () => {
		let closed = false;
		const handleClose = () => {
			closed = true;
		};
		render(Modal, {
			open: true,
			title: 'Modal Title',
			onClose: handleClose,
			children: childrenSnippet
		});
		const closeButton = screen.getByRole('button', { name: 'Close' });
		await fireEvent.click(closeButton);
		expect(closed).toBe(true);
	});

	it('renders children content (use createRawSnippet)', () => {
		render(Modal, {
			open: true,
			onClose,
			children: childrenSnippet
		});
		expect(screen.getByText('Modal content')).toBeInTheDocument();
	});
});
