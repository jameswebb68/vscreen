import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import HeroSection from './HeroSection.svelte';

describe('HeroSection', () => {
	it('renders the title', () => {
		render(HeroSection, { title: 'Breaking News' });
		expect(screen.getByText('Breaking News')).toBeInTheDocument();
	});

	it('renders the title as an h1', () => {
		render(HeroSection, { title: 'Main Title' });
		const heading = screen.getByRole('heading', { level: 1 });
		expect(heading).toHaveTextContent('Main Title');
	});

	it('renders the subtitle when provided', () => {
		render(HeroSection, { title: 'Title', subtitle: 'A subtitle' });
		expect(screen.getByText('A subtitle')).toBeInTheDocument();
	});

	it('does not render subtitle when absent', () => {
		render(HeroSection, { title: 'Title Only' });
		const paragraphs = document.querySelectorAll('p');
		expect(paragraphs).toHaveLength(0);
	});

	it('applies background image style when provided', () => {
		const { container } = render(HeroSection, {
			title: 'Hero',
			backgroundImage: 'https://img.com/bg.jpg'
		});
		const section = container.querySelector('section');
		expect(section?.getAttribute('style')).toContain('img.com/bg.jpg');
	});

	it('shows overlay when background image is set', () => {
		const { container } = render(HeroSection, {
			title: 'Hero',
			backgroundImage: 'https://img.com/bg.jpg'
		});
		const overlay = container.querySelector('.bg-black\\/60');
		expect(overlay).toBeInTheDocument();
	});

	it('does not show overlay without background image', () => {
		const { container } = render(HeroSection, { title: 'No BG' });
		const overlay = container.querySelector('.bg-black\\/60');
		expect(overlay).not.toBeInTheDocument();
	});

	it('uses a section element', () => {
		const { container } = render(HeroSection, { title: 'Test' });
		expect(container.querySelector('section')).toBeInTheDocument();
	});
});
