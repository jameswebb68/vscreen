import { loadAllPages } from '$lib/server/pages.js';

const count = loadAllPages();
if (count > 0) {
	console.log(`[synthesis] loaded ${count} persisted page(s) from .data/`);
}
