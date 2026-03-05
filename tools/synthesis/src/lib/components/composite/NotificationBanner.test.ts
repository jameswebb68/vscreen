import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import NotificationBanner from './NotificationBanner.svelte';

describe('NotificationBanner', () => {
	it('renders message', () => {
		render(NotificationBanner, { message: 'Hello world' });
		expect(screen.getByText('Hello world')).toBeInTheDocument();
	});

	it('applies info styling by default', () => {
		const { container } = render(NotificationBanner, {
			message: 'Info message'
		});
		const banner = container.querySelector('.border-blue-500\\/40');
		expect(banner).toBeInTheDocument();
	});

	it('applies error styling for error type', () => {
		const { container } = render(NotificationBanner, {
			message: 'Error message',
			type: 'error'
		});
		const banner = container.querySelector('.border-red-500\\/40');
		expect(banner).toBeInTheDocument();
	});

	it('shows dismiss button when dismissible', () => {
		render(NotificationBanner, {
			message: 'Dismissible',
			dismissible: true
		});
		expect(screen.getByRole('button', { name: 'Dismiss' })).toBeInTheDocument();
	});

	it('hides when dismiss button clicked', async () => {
		render(NotificationBanner, {
			message: 'Will hide',
			dismissible: true
		});
		expect(screen.getByText('Will hide')).toBeInTheDocument();
		const dismissButton = screen.getByRole('button', { name: 'Dismiss' });
		await fireEvent.click(dismissButton);
		expect(screen.queryByText('Will hide')).not.toBeInTheDocument();
	});
});
