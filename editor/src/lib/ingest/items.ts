import { subscribe } from '@stump/ui/graphql/client';
import {
	IngestItemEventsDocument,
	type IngestItemEventsSubscription,
	type IngestItemEventsSubscriptionVariables
} from '$lib/graphql/generated/graphql';

export type IngestItemEvent = IngestItemEventsSubscription['ingestEvents'];

export interface ItemSubscriptionOptions {
	variables: IngestItemEventsSubscriptionVariables;
	onEvent: (event: IngestItemEvent) => void;
	onError?: (message: string) => void;
}

/**
 * Follow drop-item row changes for a library. The server emits one event per
 * persisted revision — a status transition, an attached quality report, a
 * preprocess rewrite — so a consumer can invalidate a cached item without
 * knowing which column moved.
 */
export function startIngestItemSubscription(options: ItemSubscriptionOptions): () => void {
	return subscribe(IngestItemEventsDocument, options.variables, {
		next: (result) => {
			if (result.data?.ingestEvents) options.onEvent(result.data.ingestEvents);
		},
		error: () => options.onError?.('The live drop-queue connection is unavailable.'),
		complete: () => undefined
	});
}
