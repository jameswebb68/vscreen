import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types.js';
import { createPage, listPages } from '$lib/server/pages.js';
import { broadcast } from '$lib/server/ws.js';
import {
	CreatePageSchema,
	validateSections,
	formatValidationErrors,
	resolveAliasesInSections
} from '$lib/server/schemas.js';

export const GET: RequestHandler = () => {
	return json(listPages());
};

export const POST: RequestHandler = async ({ request }) => {
	const raw = await request.json();

	const parsed = CreatePageSchema.safeParse(raw);
	if (!parsed.success) {
		const msg = parsed.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`).join('; ');
		return json({ error: msg }, { status: 400 });
	}

	const body = parsed.data;

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

	const page = createPage(body);
	broadcast(page.id, 'created');
	return json(page, { status: 201 });
};
