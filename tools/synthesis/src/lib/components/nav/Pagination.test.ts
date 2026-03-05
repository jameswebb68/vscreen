import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import Pagination from './Pagination.svelte';

describe('Pagination', () => {
	const onPageChange = (_page: number) => {
		/* noop for spy */
	};

	it('renders current page indicator', () => {
		render(Pagination, {
			currentPage: 3,
			totalPages: 10,
			onPageChange
		});
		const page3Button = screen.getByRole('button', { name: '3' });
		expect(page3Button).toBeInTheDocument();
		expect(page3Button.className).toContain('bg-accent-500/20');
	});

	it('disables Prev button on first page', () => {
		render(Pagination, {
			currentPage: 1,
			totalPages: 5,
			onPageChange
		});
		const prevButton = screen.getByRole('button', { name: 'Prev' });
		expect(prevButton).toBeDisabled();
	});

	it('disables Next button on last page', () => {
		render(Pagination, {
			currentPage: 5,
			totalPages: 5,
			onPageChange
		});
		const nextButton = screen.getByRole('button', { name: 'Next' });
		expect(nextButton).toBeDisabled();
	});

	it('calls onPageChange when clicking page number', async () => {
		let changedPage: number | null = null;
		const handleChange = (page: number) => {
			changedPage = page;
		};
		render(Pagination, {
			currentPage: 2,
			totalPages: 5,
			onPageChange: handleChange
		});
		const page3Button = screen.getByRole('button', { name: '3' });
		await fireEvent.click(page3Button);
		expect(changedPage).toBe(3);
	});

	it('calls onPageChange when clicking Next', async () => {
		let changedPage: number | null = null;
		const handleChange = (page: number) => {
			changedPage = page;
		};
		render(Pagination, {
			currentPage: 2,
			totalPages: 5,
			onPageChange: handleChange
		});
		const nextButton = screen.getByRole('button', { name: 'Next' });
		await fireEvent.click(nextButton);
		expect(changedPage).toBe(3);
	});

	it('shows ellipsis for large page ranges', () => {
		const { container } = render(Pagination, {
			currentPage: 5,
			totalPages: 10,
			onPageChange
		});
		const ellipsis = container.querySelector('span.text-surface-600');
		expect(ellipsis?.textContent).toBe('…');
	});
});
