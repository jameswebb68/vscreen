import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types.js';
import { getPage, updatePage, deletePage } from '$lib/server/pages.js';
import { broadcast } from '$lib/server/ws.js';
import { validateSections, formatValidationErrors, resolveAliasesInSections } from '$lib/server/schemas.js';

export const GET: RequestHandler = ({ params }) => {
	const page = getPage(params.id);
	if (!page) {
		return json({ error: 'Page not found' }, { status: 404 });
	}
	return json(page);
};

export const PUT: RequestHandler = async ({ params, request }) => {
	const body = await request.json();

	if (body.sections && Array.isArray(body.sections) && body.sections.length > 0) {
		resolveAliasesInSections(body.sections);
		const sectionErrors = validateSections(body.sections);
		if (sectionErrors.length > 0) {
			return json(
				{ error: 'Section validation failed', details: formatValidationErrors(sectionErrors) },
				{ status: 400 }
			);
		}
	}

	const page = updatePage(params.id, body);
	if (!page) {
		return json({ error: 'Page not found' }, { status: 404 });
	}
	broadcast(page.id, 'updated');
	return json(page);
};

export const DELETE: RequestHandler = ({ params }) => {
	const deleted = deletePage(params.id);
	if (!deleted) {
		return json({ error: 'Page not found' }, { status: 404 });
	}
	broadcast(params.id, 'deleted');
	return json({ ok: true });
};
