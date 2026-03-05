import { describe, it, expect, vi } from 'vitest';
import { subscribe, broadcast } from './ws.js';

describe('ws broadcast hub', () => {
	// -----------------------------------------------------------------------
	// subscribe
	// -----------------------------------------------------------------------

	describe('subscribe', () => {
		it('returns an unsubscribe function', () => {
			const unsub = subscribe(() => {});
			expect(typeof unsub).toBe('function');
			unsub();
		});

		it('listener receives broadcast events', () => {
			const listener = vi.fn();
			const unsub = subscribe(listener);

			broadcast('page-1', 'created');

			expect(listener).toHaveBeenCalledTimes(1);
			expect(listener).toHaveBeenCalledWith('page-1', 'created');

			unsub();
		});

		it('multiple listeners all receive the same event', () => {
			const l1 = vi.fn();
			const l2 = vi.fn();
			const l3 = vi.fn();

			const u1 = subscribe(l1);
			const u2 = subscribe(l2);
			const u3 = subscribe(l3);

			broadcast('page-x', 'updated');

			expect(l1).toHaveBeenCalledWith('page-x', 'updated');
			expect(l2).toHaveBeenCalledWith('page-x', 'updated');
			expect(l3).toHaveBeenCalledWith('page-x', 'updated');

			u1();
			u2();
			u3();
		});
	});

	// -----------------------------------------------------------------------
	// unsubscribe
	// -----------------------------------------------------------------------

	describe('unsubscribe', () => {
		it('stops receiving events after unsubscribe', () => {
			const listener = vi.fn();
			const unsub = subscribe(listener);

			broadcast('page-1', 'created');
			expect(listener).toHaveBeenCalledTimes(1);

			unsub();

			broadcast('page-1', 'updated');
			expect(listener).toHaveBeenCalledTimes(1);
		});

		it('unsubscribe is idempotent', () => {
			const listener = vi.fn();
			const unsub = subscribe(listener);

			unsub();
			unsub();
			unsub();

			broadcast('page-1', 'test');
			expect(listener).not.toHaveBeenCalled();
		});

		it('only removes the specific listener', () => {
			const staying = vi.fn();
			const leaving = vi.fn();

			const u1 = subscribe(staying);
			const u2 = subscribe(leaving);

			u2();

			broadcast('page-1', 'push');
			expect(staying).toHaveBeenCalledTimes(1);
			expect(leaving).not.toHaveBeenCalled();

			u1();
		});
	});

	// -----------------------------------------------------------------------
	// broadcast
	// -----------------------------------------------------------------------

	describe('broadcast', () => {
		it('does nothing when there are no listeners', () => {
			expect(() => broadcast('page-1', 'created')).not.toThrow();
		});

		it('delivers the correct pageId and event type', () => {
			const events: Array<{ pageId: string; event: string }> = [];
			const unsub = subscribe((pageId, event) => {
				events.push({ pageId, event });
			});

			broadcast('news-digest', 'created');
			broadcast('dashboard', 'updated');
			broadcast('news-digest', 'push');
			broadcast('old-page', 'deleted');

			expect(events).toEqual([
				{ pageId: 'news-digest', event: 'created' },
				{ pageId: 'dashboard', event: 'updated' },
				{ pageId: 'news-digest', event: 'push' },
				{ pageId: 'old-page', event: 'deleted' }
			]);

			unsub();
		});

		it('handles listener throwing without affecting others', () => {
			const good = vi.fn();
			const bad = vi.fn(() => {
				throw new Error('boom');
			});

			const u1 = subscribe(bad);
			const u2 = subscribe(good);

			expect(() => broadcast('page-1', 'test')).toThrow('boom');

			u1();
			u2();
		});
	});
});
