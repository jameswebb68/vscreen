import type { PageServerLoad } from './$types.js';
import { listPages } from '$lib/server/pages.js';

export const load: PageServerLoad = () => {
	return { pages: listPages() };
};
