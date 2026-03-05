import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type {
	SynthesisPage,
	CreatePageRequest,
	UpdatePageRequest,
	Section,
	SectionData
} from '$lib/types/index.js';

const pages = new Map<string, SynthesisPage>();

const DATA_DIR = resolve(process.cwd(), '.data');

function slugify(title: string): string {
	return title
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, '-')
		.replace(/^-|-$/g, '');
}

function uniqueId(title: string): string {
	const base = slugify(title);
	if (!pages.has(base)) return base;
	let i = 2;
	while (pages.has(`${base}-${i}`)) i++;
	return `${base}-${i}`;
}

export function createPage(req: CreatePageRequest): SynthesisPage {
	const now = new Date().toISOString();
	const id = uniqueId(req.title);
	const page: SynthesisPage = {
		id,
		title: req.title,
		subtitle: req.subtitle,
		theme: req.theme ?? 'dark',
		accentColor: req.accentColor,
		layout: req.layout ?? 'grid',
		sections: req.sections ?? [],
		createdAt: now,
		updatedAt: now
	};
	pages.set(id, page);
	return page;
}

export function getPage(id: string): SynthesisPage | undefined {
	return pages.get(id);
}

export function listPages(): SynthesisPage[] {
	return [...pages.values()];
}

export function updatePage(id: string, req: UpdatePageRequest): SynthesisPage | undefined {
	const page = pages.get(id);
	if (!page) return undefined;

	if (req.title !== undefined) page.title = req.title;
	if (req.subtitle !== undefined) page.subtitle = req.subtitle;
	if (req.theme !== undefined) page.theme = req.theme;
	if (req.accentColor !== undefined) page.accentColor = req.accentColor;
	if (req.layout !== undefined) page.layout = req.layout;
	if (req.sections !== undefined) page.sections = req.sections;
	page.updatedAt = new Date().toISOString();

	return page;
}

export function deletePage(id: string): boolean {
	return pages.delete(id);
}

export function pushData(
	pageId: string,
	sectionId: string,
	data: SectionData
): Section | undefined {
	const page = pages.get(pageId);
	if (!page) return undefined;

	let section = page.sections.find((s) => s.id === sectionId);
	if (section) {
		section.data = [...section.data, ...data] as SectionData;
	} else {
		section = { id: sectionId, component: 'card-grid', data };
		page.sections.push(section);
	}
	page.updatedAt = new Date().toISOString();
	return section;
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

export function savePage(id: string): boolean {
	const page = pages.get(id);
	if (!page) return false;
	if (!existsSync(DATA_DIR)) mkdirSync(DATA_DIR, { recursive: true });
	writeFileSync(join(DATA_DIR, `${id}.json`), JSON.stringify(page, null, 2), 'utf-8');
	return true;
}

export function loadAllPages(): number {
	if (!existsSync(DATA_DIR)) return 0;
	let count = 0;
	for (const file of readdirSync(DATA_DIR)) {
		if (!file.endsWith('.json')) continue;
		try {
			const raw = readFileSync(join(DATA_DIR, file), 'utf-8');
			const page = JSON.parse(raw) as SynthesisPage;
			if (page.id) {
				pages.set(page.id, page);
				count++;
			}
		} catch {
			// skip malformed files
		}
	}
	return count;
}

export function getDataDir(): string {
	return DATA_DIR;
}
