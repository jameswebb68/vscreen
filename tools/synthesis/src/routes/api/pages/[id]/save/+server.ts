import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types.js';
import { savePage } from '$lib/server/pages.js';

export const POST: RequestHandler = ({ params }) => {
	const saved = savePage(params.id);
	if (!saved) {
		return json({ error: 'Page not found' }, { status: 404 });
	}
	return json({ ok: true, id: params.id });
};
