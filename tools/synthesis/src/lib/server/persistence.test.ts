import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { existsSync, mkdirSync, rmSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createPage, getPage, savePage, loadAllPages, getDataDir, deletePage, listPages } from './pages.js';

describe('persistence', () => {
	const dataDir = getDataDir();

	beforeEach(() => {
		// Clean in-memory state by deleting all pages
		for (const p of listPages()) {
			deletePage(p.id);
		}
		// Clean data dir
		if (existsSync(dataDir)) rmSync(dataDir, { recursive: true });
	});

	afterEach(() => {
		if (existsSync(dataDir)) rmSync(dataDir, { recursive: true });
	});

	it('savePage creates .data directory if missing', () => {
		createPage({ title: 'Test Page' });
		savePage('test-page');
		expect(existsSync(dataDir)).toBe(true);
	});

	it('savePage writes valid JSON to .data/{id}.json', () => {
		const page = createPage({ title: 'Save Me' });
		savePage(page.id);
		const filePath = join(dataDir, `${page.id}.json`);
		expect(existsSync(filePath)).toBe(true);
		const raw = readFileSync(filePath, 'utf-8');
		const parsed = JSON.parse(raw);
		expect(parsed.title).toBe('Save Me');
		expect(parsed.id).toBe(page.id);
	});

	it('savePage returns false for non-existent page', () => {
		expect(savePage('nonexistent')).toBe(false);
	});

	it('savePage returns true for existing page', () => {
		createPage({ title: 'Exists' });
		expect(savePage('exists')).toBe(true);
	});

	it('loadAllPages loads saved pages into memory', () => {
		const page = createPage({ title: 'Persisted Page', theme: 'light' });
		savePage(page.id);
		deletePage(page.id);
		expect(getPage(page.id)).toBeUndefined();

		const count = loadAllPages();
		expect(count).toBe(1);
		const loaded = getPage(page.id);
		expect(loaded).toBeDefined();
		expect(loaded?.title).toBe('Persisted Page');
		expect(loaded?.theme).toBe('light');
	});

	it('loadAllPages returns 0 when .data does not exist', () => {
		expect(loadAllPages()).toBe(0);
	});

	it('loadAllPages skips malformed JSON files', () => {
		mkdirSync(dataDir, { recursive: true });
		writeFileSync(join(dataDir, 'bad.json'), 'not json!', 'utf-8');
		expect(loadAllPages()).toBe(0);
	});

	it('loadAllPages skips files without .json extension', () => {
		mkdirSync(dataDir, { recursive: true });
		writeFileSync(join(dataDir, 'readme.txt'), 'hello', 'utf-8');
		expect(loadAllPages()).toBe(0);
	});

	it('loadAllPages loads multiple pages', () => {
		createPage({ title: 'Page A' });
		createPage({ title: 'Page B' });
		savePage('page-a');
		savePage('page-b');
		deletePage('page-a');
		deletePage('page-b');

		const count = loadAllPages();
		expect(count).toBe(2);
		expect(getPage('page-a')).toBeDefined();
		expect(getPage('page-b')).toBeDefined();
	});

	it('savePage preserves sections and metadata', () => {
		const page = createPage({
			title: 'With Sections',
			sections: [{ id: 'sec1', component: 'card-grid', data: [{ title: 'Card 1' }] }]
		});
		savePage(page.id);
		deletePage(page.id);
		loadAllPages();
		const loaded = getPage(page.id);
		expect(loaded?.sections).toHaveLength(1);
		expect(loaded?.sections[0]?.id).toBe('sec1');
	});
});
