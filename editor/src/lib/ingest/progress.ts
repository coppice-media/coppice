import { subscribe } from '@stump/ui/graphql/client';
import {
	IngestProgressDocument,
	type IngestProgressSubscription,
	type IngestProgressSubscriptionVariables
} from '$lib/graphql/generated/graphql';

export type ProgressEvent = IngestProgressSubscription['ingestProgress'];

export interface ProgressPatch {
	status: ProgressEvent['status'];
	phase: ProgressEvent['phase'];
	completed: number;
	total: number;
	score: number | null;
	message: string | null;
	eventId: string;
	emittedAt: string;
}

export interface ProgressState {
	cursor: string | null;
	cursorExpired: boolean;
	items: Record<string, ProgressPatch>;
	jobs: Record<string, ProgressPatch>;
}

export type ProgressAction =
	| { type: 'EVENT'; event: ProgressEvent }
	| { type: 'CURSOR_EXPIRED' }
	| { type: 'RESET' };

export function initialProgressState(): ProgressState {
	return {
		cursor: null,
		cursorExpired: false,
		items: {},
		jobs: {}
	};
}

export function progressReducer(state: ProgressState, action: ProgressAction): ProgressState {
	switch (action.type) {
		case 'RESET':
			return initialProgressState();
		case 'CURSOR_EXPIRED':
			return { ...state, cursor: null, cursorExpired: true };
		case 'EVENT': {
			const { event } = action;
			const patch: ProgressPatch = {
			status: event.status,
			phase: event.phase,
			completed: event.completed,
			total: event.total,
			score: event.score,
			message: event.message,
			eventId: event.eventId,
			emittedAt: event.emittedAt
		};
		return {
			cursor: event.eventId,
			cursorExpired: false,
			items: { ...state.items, [event.dropItemId]: patch },
			jobs: event.analysisJobId
				? { ...state.jobs, [event.analysisJobId]: patch }
				: state.jobs
		};
		}
	}
}

export interface ProgressSubscriptionOptions {
	variables: IngestProgressSubscriptionVariables;
	onEvent: (event: ProgressEvent) => void;
	onCursorExpired: () => void;
	onError?: (message: string) => void;
}

function isCursorExpired(value: unknown): boolean {
	return JSON.stringify(value).toUpperCase().includes('CURSOR_EXPIRED');
}

export function startIngestProgressSubscription(options: ProgressSubscriptionOptions): () => void {
	return subscribe(IngestProgressDocument, options.variables, {
		next: (result) => {
			if (result.errors?.some(isCursorExpired)) {
				options.onCursorExpired();
				return;
			}
			if (result.data?.ingestProgress) options.onEvent(result.data.ingestProgress);
		},
		error: (error) => {
			if (isCursorExpired(error)) options.onCursorExpired();
			else options.onError?.('The live progress connection is unavailable.');
		},
		complete: () => undefined
	});
}
