import { describe, it, expect, beforeEach } from 'vitest';
import {
	createPage,
	getPage,
	listPages,
	updatePage,
	deletePage,
	pushData
} from './pages.js';
import type { ArticleItem, StatItem, Section } from '$lib/types/index.js';

/**
 * Reset the in-memory store between tests by deleting all pages.
 * The store is module-level state, so we clean it manually.
 */
function clearAllPages(): void {
	for (const page of listPages()) {
		deletePage(page.id);
	}
}

describe('page store', () => {
	beforeEach(() => {
		clearAllPages();
	});

	// -----------------------------------------------------------------------
	// createPage
	// -----------------------------------------------------------------------

	describe('createPage', () => {
		it('creates a page with required fields', () => {
			const page = createPage({ title: 'Test Page' });
			expect(page.id).toBe('test-page');
			expect(page.title).toBe('Test Page');
			expect(page.theme).toBe('dark');
			expect(page.layout).toBe('grid');
			expect(page.sections).toEqual([]);
			expect(page.createdAt).toBeTruthy();
			expect(page.updatedAt).toBeTruthy();
		});

		it('creates a page with all optional fields', () => {
			const sections: Section[] = [
				{ id: 'sec1', component: 'card-grid', title: 'Section 1', data: [] }
			];
			const page = createPage({
				title: 'Full Page',
				subtitle: 'A subtitle',
				theme: 'light',
				accentColor: '#ff0000',
				layout: 'split',
				sections
			});
			expect(page.subtitle).toBe('A subtitle');
			expect(page.theme).toBe('light');
			expect(page.accentColor).toBe('#ff0000');
			expect(page.layout).toBe('split');
			expect(page.sections).toHaveLength(1);
		});

		it('stores the page in the internal map', () => {
			const page = createPage({ title: 'Stored Page' });
			const fetched = getPage(page.id);
			expect(fetched).toBeDefined();
			expect(fetched?.title).toBe('Stored Page');
		});

		it('sets createdAt and updatedAt to the same ISO timestamp', () => {
			const page = createPage({ title: 'Timestamps' });
			expect(page.createdAt).toBe(page.updatedAt);
			expect(() => new Date(page.createdAt)).not.toThrow();
		});
	});

	// -----------------------------------------------------------------------
	// slugify / uniqueId
	// -----------------------------------------------------------------------

	describe('slug generation', () => {
		it('lowercases and hyphenates the title', () => {
			const page = createPage({ title: 'Hello World' });
			expect(page.id).toBe('hello-world');
		});

		it('strips non-alphanumeric characters', () => {
			const page = createPage({ title: 'CNN: Breaking News!!!' });
			expect(page.id).toBe('cnn-breaking-news');
		});

		it('strips leading and trailing hyphens', () => {
			const page = createPage({ title: '---test---' });
			expect(page.id).toBe('test');
		});

		it('handles Unicode by stripping non-ASCII', () => {
			const page = createPage({ title: 'café résumé' });
			expect(page.id).toBe('caf-r-sum');
		});

		it('generates unique IDs for duplicate titles', () => {
			const p1 = createPage({ title: 'Duplicate' });
			const p2 = createPage({ title: 'Duplicate' });
			const p3 = createPage({ title: 'Duplicate' });
			expect(p1.id).toBe('duplicate');
			expect(p2.id).toBe('duplicate-2');
			expect(p3.id).toBe('duplicate-3');
		});

		it('handles empty-ish titles', () => {
			const page = createPage({ title: '!!!' });
			expect(page.id).toBe('');
		});
	});

	// -----------------------------------------------------------------------
	// getPage
	// -----------------------------------------------------------------------

	describe('getPage', () => {
		it('returns the page when it exists', () => {
			createPage({ title: 'Existing' });
			const page = getPage('existing');
			expect(page).toBeDefined();
			expect(page?.title).toBe('Existing');
		});

		it('returns undefined for a nonexistent ID', () => {
			expect(getPage('nonexistent')).toBeUndefined();
		});
	});

	// -----------------------------------------------------------------------
	// listPages
	// -----------------------------------------------------------------------

	describe('listPages', () => {
		it('returns empty array when no pages exist', () => {
			expect(listPages()).toEqual([]);
		});

		it('returns all created pages', () => {
			createPage({ title: 'Page A' });
			createPage({ title: 'Page B' });
			createPage({ title: 'Page C' });
			const pages = listPages();
			expect(pages).toHaveLength(3);
			const ids = pages.map((p) => p.id);
			expect(ids).toContain('page-a');
			expect(ids).toContain('page-b');
			expect(ids).toContain('page-c');
		});

		it('returns a snapshot (not a live reference)', () => {
			createPage({ title: 'Snapshot Test' });
			const list1 = listPages();
			createPage({ title: 'After Snapshot' });
			const list2 = listPages();
			expect(list1).toHaveLength(1);
			expect(list2).toHaveLength(2);
		});
	});

	// -----------------------------------------------------------------------
	// updatePage
	// -----------------------------------------------------------------------

	describe('updatePage', () => {
		it('returns undefined for a nonexistent page', () => {
			expect(updatePage('nope', { title: 'New' })).toBeUndefined();
		});

		it('updates the title', () => {
			createPage({ title: 'Old Title' });
			const updated = updatePage('old-title', { title: 'New Title' });
			expect(updated?.title).toBe('New Title');
		});

		it('updates the theme', () => {
			createPage({ title: 'Theme Test' });
			const updated = updatePage('theme-test', { theme: 'light' });
			expect(updated?.theme).toBe('light');
		});

		it('updates the layout', () => {
			createPage({ title: 'Layout Test' });
			const updated = updatePage('layout-test', { layout: 'tabs' });
			expect(updated?.layout).toBe('tabs');
		});

		it('updates the subtitle', () => {
			createPage({ title: 'Sub Test' });
			const updated = updatePage('sub-test', { subtitle: 'A new subtitle' });
			expect(updated?.subtitle).toBe('A new subtitle');
		});

		it('updates the accentColor', () => {
			createPage({ title: 'Color Test' });
			const updated = updatePage('color-test', { accentColor: '#00ff00' });
			expect(updated?.accentColor).toBe('#00ff00');
		});

		it('replaces sections entirely', () => {
			const sections: Section[] = [
				{ id: 's1', component: 'hero', data: [] }
			];
			createPage({
				title: 'Sections Test',
				sections: [{ id: 'old', component: 'card-grid', data: [] }]
			});
			const updated = updatePage('sections-test', { sections });
			expect(updated?.sections).toHaveLength(1);
			expect(updated?.sections[0]?.id).toBe('s1');
		});

		it('advances updatedAt', async () => {
			const page = createPage({ title: 'Time Test' });
			const originalUpdatedAt = page.updatedAt;
			await new Promise((r) => setTimeout(r, 10));
			const updated = updatePage('time-test', { title: 'Time Test 2' });
			expect(updated?.updatedAt).not.toBe(originalUpdatedAt);
		});

		it('does not touch fields not in the update request', () => {
			createPage({ title: 'Partial', subtitle: 'Keep Me', theme: 'dark' });
			const updated = updatePage('partial', { title: 'Changed' });
			expect(updated?.title).toBe('Changed');
			expect(updated?.subtitle).toBe('Keep Me');
			expect(updated?.theme).toBe('dark');
		});

		it('reflects changes in subsequent getPage calls', () => {
			createPage({ title: 'Persist Test' });
			updatePage('persist-test', { theme: 'light' });
			const fetched = getPage('persist-test');
			expect(fetched?.theme).toBe('light');
		});
	});

	// -----------------------------------------------------------------------
	// deletePage
	// -----------------------------------------------------------------------

	describe('deletePage', () => {
		it('returns true when deleting an existing page', () => {
			createPage({ title: 'Delete Me' });
			expect(deletePage('delete-me')).toBe(true);
		});

		it('returns false when deleting a nonexistent page', () => {
			expect(deletePage('ghost')).toBe(false);
		});

		it('removes the page from the store', () => {
			createPage({ title: 'Gone Soon' });
			deletePage('gone-soon');
			expect(getPage('gone-soon')).toBeUndefined();
		});

		it('reduces list count', () => {
			createPage({ title: 'A' });
			createPage({ title: 'B' });
			expect(listPages()).toHaveLength(2);
			deletePage('a');
			expect(listPages()).toHaveLength(1);
		});
	});

	// -----------------------------------------------------------------------
	// pushData
	// -----------------------------------------------------------------------

	describe('pushData', () => {
		it('returns undefined for a nonexistent page', () => {
			const items: ArticleItem[] = [{ title: 'article' }];
			expect(pushData('nope', 'sec', items)).toBeUndefined();
		});

		it('appends data to an existing section', () => {
			const initial: ArticleItem[] = [{ title: 'Article 1', source: 'CNN' }];
			createPage({
				title: 'Push Test',
				sections: [{ id: 'news', component: 'card-grid', data: initial }]
			});

			const pushed: ArticleItem[] = [{ title: 'Article 2', source: 'BBC' }];
			const section = pushData('push-test', 'news', pushed);

			expect(section).toBeDefined();
			expect(section?.data).toHaveLength(2);
			expect((section?.data as ArticleItem[])[0]?.title).toBe('Article 1');
			expect((section?.data as ArticleItem[])[1]?.title).toBe('Article 2');
		});

		it('creates a new section if section_id does not exist', () => {
			createPage({ title: 'New Section' });

			const items: ArticleItem[] = [{ title: 'Brand New' }];
			const section = pushData('new-section', 'fresh', items);

			expect(section).toBeDefined();
			expect(section?.id).toBe('fresh');
			expect(section?.component).toBe('card-grid');
			expect(section?.data).toHaveLength(1);
		});

		it('auto-created section is reflected in the page', () => {
			createPage({ title: 'Reflect Test' });
			pushData('reflect-test', 'auto', [{ title: 'Item' }] as ArticleItem[]);

			const page = getPage('reflect-test');
			expect(page?.sections).toHaveLength(1);
			expect(page?.sections[0]?.id).toBe('auto');
		});

		it('advances updatedAt on push', async () => {
			const page = createPage({ title: 'Push Time' });
			const before = page.updatedAt;
			await new Promise((r) => setTimeout(r, 10));

			pushData('push-time', 'sec', [{ title: 'x' }] as ArticleItem[]);
			const after = getPage('push-time')?.updatedAt;
			expect(after).not.toBe(before);
		});

		it('handles multiple pushes to the same section', () => {
			createPage({
				title: 'Multi Push',
				sections: [{ id: 'feed', component: 'live-feed', data: [] }]
			});

			pushData('multi-push', 'feed', [{ title: 'A' }] as ArticleItem[]);
			pushData('multi-push', 'feed', [{ title: 'B' }] as ArticleItem[]);
			pushData('multi-push', 'feed', [{ title: 'C' }] as ArticleItem[]);

			const section = getPage('multi-push')?.sections.find((s) => s.id === 'feed');
			expect(section?.data).toHaveLength(3);
		});

		it('works with stat items', () => {
			createPage({
				title: 'Stats Test',
				sections: [{ id: 'stats', component: 'stats-row', data: [] }]
			});

			const stats: StatItem[] = [
				{ label: 'Users', value: 1500, trend: 'up' },
				{ label: 'Revenue', value: '$25k', unit: 'USD' }
			];
			const section = pushData('stats-test', 'stats', stats);
			expect(section?.data).toHaveLength(2);
		});
	});
});
