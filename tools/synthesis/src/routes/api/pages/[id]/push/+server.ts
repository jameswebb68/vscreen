import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types.js';
import { getPage, pushData } from '$lib/server/pages.js';
import { broadcast } from '$lib/server/ws.js';
import { z } from 'zod';

const PushSchema = z.object({
	section_id: z.string().min(1),
	data: z.array(z.any()).min(1)
});

export const POST: RequestHandler = async ({ params, request }) => {
	const raw = await request.json();

	const parsed = PushSchema.safeParse(raw);
	if (!parsed.success) {
		const msg = parsed.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`).join('; ');
		return json({ error: msg }, { status: 400 });
	}

	const { section_id, data } = parsed.data;

	const page = getPage(params.id);
	if (!page) {
		return json({ error: 'Page not found' }, { status: 404 });
	}

	const section = pushData(params.id, section_id, data);
	if (!section) {
		return json({ error: 'Page not found' }, { status: 404 });
	}

	broadcast(params.id, 'push');
	return json(section);
};
