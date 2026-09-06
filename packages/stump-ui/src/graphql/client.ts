import { browser } from '$app/environment';
import { resolve } from '$app/paths';
import { ClientError, GraphQLClient } from 'graphql-request';
import { print } from 'graphql';
import { createClient, type Sink } from 'graphql-ws';
import type { TypedDocumentNode } from '@graphql-typed-document-node/core';

export const graphQLEndpoint = browser
	? `${window.location.origin}/api/graphql`
	: '/api/graphql';

export const graphQLClient = new GraphQLClient(graphQLEndpoint, {
	credentials: 'include'
});

/**
 * Send an unauthenticated visitor to the app's login screen, preserving the
 * current location as `returnTo`. Shared by the plain GraphQL `request` helper
 * and the multipart upload helpers so 401 handling stays in lockstep.
 */
export function redirectToLogin(): void {
	if (!browser || window.location.pathname === resolve('/login')) return;
	const returnTo = `${window.location.pathname}${window.location.search}`;
	window.location.assign(`${resolve('/login')}?returnTo=${encodeURIComponent(returnTo)}`);
}

/**
 * Run a typed operation. A `401` sends the visitor to the login screen; a
 * GraphQL error surfaces as an `Error` carrying the server's first message
 * (graphql-request's own message dumps the whole request and response,
 * which is what toasts would otherwise show).
 */
export async function request<TResult, TVariables extends object>(
	document: TypedDocumentNode<TResult, TVariables>,
	variables: TVariables
): Promise<TResult> {
	try {
		return await graphQLClient.request<TResult>(print(document), variables);
	} catch (error) {
		if (error instanceof ClientError) {
			if (error.response.status === 401) redirectToLogin();
			const message = error.response.errors?.[0]?.message;
			if (message) throw new Error(message, { cause: error });
		}
		throw error;
	}
}

/**
 * Subscribe to `document` over a dedicated `graphql-ws` connection to the
 * current origin. Returns a function that tears down the subscription and
 * the connection. A no-op outside the browser.
 */
export function subscribe<TResult, TVariables extends Record<string, unknown>>(
	document: TypedDocumentNode<TResult, TVariables>,
	variables: TVariables,
	sink: Sink<{ data?: TResult | null; errors?: readonly unknown[] }>
): () => void {
	if (!browser) return () => undefined;
	const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
	const client = createClient({
		url: `${protocol}//${window.location.host}/api/graphql/ws`,
		connectionParams: {},
		retryAttempts: 3,
		lazy: true,
		keepAlive: 12_000
	});
	const dispose = client.subscribe<TResult>({ query: print(document), variables }, sink);
	return () => {
		dispose();
		client.dispose();
	};
}
