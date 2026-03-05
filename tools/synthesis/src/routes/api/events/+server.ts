import type { RequestHandler } from './$types.js';
import { subscribe } from '$lib/server/ws.js';

export const GET: RequestHandler = () => {
	let unsubscribe: (() => void) | undefined;

	const stream = new ReadableStream({
		start(controller) {
			const encoder = new TextEncoder();

			const send = (data: string) => {
				controller.enqueue(encoder.encode(`data: ${data}\n\n`));
			};

			send(JSON.stringify({ type: 'connected' }));

			unsubscribe = subscribe((pageId, event) => {
				try {
					send(JSON.stringify({ type: event, pageId }));
				} catch {
					unsubscribe?.();
				}
			});
		},
		cancel() {
			unsubscribe?.();
		}
	});

	return new Response(stream, {
		headers: {
			'Content-Type': 'text/event-stream',
			'Cache-Control': 'no-cache',
			Connection: 'keep-alive'
		}
	});
};
