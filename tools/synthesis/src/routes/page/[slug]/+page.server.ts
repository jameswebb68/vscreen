import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types.js';
import { getPage } from '$lib/server/pages.js';

export const load: PageServerLoad = ({ params }) => {
	const page = getPage(params.slug);
	if (!page) {
		error(404, 'Page not found');
	}
	return { page };
};
