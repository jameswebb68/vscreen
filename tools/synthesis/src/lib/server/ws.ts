type Listener = (pageId: string, event: string) => void;

const listeners = new Set<Listener>();

export function subscribe(listener: Listener): () => void {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

export function broadcast(pageId: string, event: string): void {
	for (const listener of listeners) {
		listener(pageId, event);
	}
}
