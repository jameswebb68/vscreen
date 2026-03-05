import { describe, it, expect, beforeEach } from 'vitest';
import {
	createPage,
	getPage,
	listPages,
	deletePage,
	updatePage,
	pushData
} from './pages.js';
import { subscribe, broadcast } from './ws.js';
import type { ArticleItem } from '$lib/types/index.js';

/**
 * These tests exercise the page store + broadcast integration
 * that backs the API routes. We test the data layer directly
 * since SvelteKit route handlers are thin wrappers around these functions.
 */

function clearAllPages(): void {
	for (const page of listPages()) {
		deletePage(page.id);
	}
}

describe('API data layer integration', () => {
	beforeEach(() => {
		clearAllPages();
	});

	// -----------------------------------------------------------------------
	// POST /api/pages (createPage + broadcast)
	// -----------------------------------------------------------------------

	describe('create page flow', () => {
		it('creates a page and broadcasts "created"', () => {
			const events: Array<{ pageId: string; event: string }> = [];
			const unsub = subscribe((pageId, event) => {
				events.push({ pageId, event });
			});

			const page = createPage({ title: 'News Feed' });

			expect(page.id).toBe('news-feed');
			expect(listPages()).toHaveLength(1);

			unsub();
		});

		it('defaults theme to dark and layout to grid', () => {
			const page = createPage({ title: 'Defaults' });
			expect(page.theme).toBe('dark');
			expect(page.layout).toBe('grid');
		});

		it('respects provided theme and layout', () => {
			const page = createPage({ title: 'Custom', theme: 'light', layout: 'tabs' });
			expect(page.theme).toBe('light');
			expect(page.layout).toBe('tabs');
		});
	});

	// -----------------------------------------------------------------------
	// GET /api/pages (listPages)
	// -----------------------------------------------------------------------

	describe('list pages flow', () => {
		it('returns empty array when no pages exist', () => {
			expect(listPages()).toEqual([]);
		});

		it('returns all pages ordered by creation', () => {
			createPage({ title: 'Alpha' });
			createPage({ title: 'Beta' });
			const pages = listPages();
			expect(pages).toHaveLength(2);
		});
	});

	// -----------------------------------------------------------------------
	// GET /api/pages/:id (getPage)
	// -----------------------------------------------------------------------

	describe('get page flow', () => {
		it('returns page for valid ID', () => {
			createPage({ title: 'Get Me' });
			const page = getPage('get-me');
			expect(page).toBeDefined();
			expect(page?.title).toBe('Get Me');
		});

		it('returns undefined for 404', () => {
			expect(getPage('nonexistent-id')).toBeUndefined();
		});
	});

	// -----------------------------------------------------------------------
	// PUT /api/pages/:id (updatePage + broadcast)
	// -----------------------------------------------------------------------

	describe('update page flow', () => {
		it('updates and returns the modified page', () => {
			createPage({ title: 'Before' });
			const updated = updatePage('before', { title: 'After', theme: 'light' });
			expect(updated?.title).toBe('After');
			expect(updated?.theme).toBe('light');
		});

		it('returns undefined for nonexistent page', () => {
			expect(updatePage('ghost', { title: 'Nope' })).toBeUndefined();
		});

		it('partial update preserves other fields', () => {
			createPage({ title: 'Keep Fields', subtitle: 'Important' });
			updatePage('keep-fields', { theme: 'light' });
			const page = getPage('keep-fields');
			expect(page?.subtitle).toBe('Important');
			expect(page?.theme).toBe('light');
		});
	});

	// -----------------------------------------------------------------------
	// DELETE /api/pages/:id (deletePage + broadcast)
	// -----------------------------------------------------------------------

	describe('delete page flow', () => {
		it('deletes existing page and returns true', () => {
			createPage({ title: 'Doomed' });
			expect(deletePage('doomed')).toBe(true);
			expect(getPage('doomed')).toBeUndefined();
		});

		it('returns false for nonexistent page', () => {
			expect(deletePage('never-existed')).toBe(false);
		});
	});

	// -----------------------------------------------------------------------
	// POST /api/pages/:id/push (pushData + broadcast)
	// -----------------------------------------------------------------------

	describe('push data flow', () => {
		it('appends to existing section', () => {
			const initial: ArticleItem[] = [{ title: 'First' }];
			createPage({
				title: 'Push Flow',
				sections: [{ id: 'feed', component: 'card-grid', data: initial }]
			});

			const added: ArticleItem[] = [{ title: 'Second' }];
			const section = pushData('push-flow', 'feed', added);

			expect(section?.data).toHaveLength(2);
		});

		it('creates section if missing', () => {
			createPage({ title: 'Auto Section' });
			const section = pushData('auto-section', 'new-feed', [
				{ title: 'Item' }
			] as ArticleItem[]);

			expect(section?.id).toBe('new-feed');
			expect(section?.component).toBe('card-grid');
		});

		it('returns undefined for missing page', () => {
			expect(pushData('ghost', 'feed', [{ title: 'x' }] as ArticleItem[])).toBeUndefined();
		});
	});

	// -----------------------------------------------------------------------
	// End-to-end scenario: news aggregator
	// -----------------------------------------------------------------------

	describe('end-to-end: news aggregator', () => {
		it('creates a multi-source news page and pushes updates', () => {
			const cnnArticles: ArticleItem[] = [
				{ title: 'CNN Story 1', source: 'CNN', url: 'https://cnn.com/1' },
				{ title: 'CNN Story 2', source: 'CNN', url: 'https://cnn.com/2' }
			];
			const bbcArticles: ArticleItem[] = [
				{ title: 'BBC Story 1', source: 'BBC', url: 'https://bbc.com/1' },
				{ title: 'BBC Story 2', source: 'BBC', url: 'https://bbc.com/2' }
			];

			const page = createPage({
				title: 'News Digest',
				theme: 'dark',
				layout: 'grid',
				sections: [
					{ id: 'cnn', component: 'card-grid', title: 'CNN', data: cnnArticles },
					{ id: 'bbc', component: 'card-grid', title: 'BBC', data: bbcArticles }
				]
			});

			expect(page.id).toBe('news-digest');
			expect(page.sections).toHaveLength(2);
			expect(page.sections[0]?.data).toHaveLength(2);
			expect(page.sections[1]?.data).toHaveLength(2);

			const newArticle: ArticleItem[] = [
				{ title: 'Breaking CNN', source: 'CNN', url: 'https://cnn.com/3' }
			];
			pushData('news-digest', 'cnn', newArticle);

			const updated = getPage('news-digest');
			expect(updated?.sections[0]?.data).toHaveLength(3);
			expect(updated?.sections[1]?.data).toHaveLength(2);

			updatePage('news-digest', { theme: 'light' });
			expect(getPage('news-digest')?.theme).toBe('light');

			expect(deletePage('news-digest')).toBe(true);
			expect(getPage('news-digest')).toBeUndefined();
		});
	});

	// -----------------------------------------------------------------------
	// Broadcast integration
	// -----------------------------------------------------------------------

	describe('broadcast integration', () => {
		it('broadcast fires for create, push, update, delete', () => {
			const events: string[] = [];
			const unsub = subscribe((_pageId, event) => {
				events.push(event);
			});

			const page = createPage({ title: 'Broadcast Test' });

			broadcast(page.id, 'created');
			pushData(page.id, 'feed', [{ title: 'item' }] as ArticleItem[]);
			broadcast(page.id, 'push');
			updatePage(page.id, { theme: 'light' });
			broadcast(page.id, 'updated');
			deletePage(page.id);
			broadcast(page.id, 'deleted');

			expect(events).toEqual(['created', 'push', 'updated', 'deleted']);

			unsub();
		});
	});
});
