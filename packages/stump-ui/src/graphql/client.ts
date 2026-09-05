import { browser } from '$app/environment';
import { resolve } from '$app/paths';
import { ClientError, GraphQLClient } from 'graphql-request';
import { print } from 'graphql';
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

export async function request<TResult, TVariables extends object>(
	document: TypedDocumentNode<TResult, TVariables>,
	variables: TVariables
): Promise<TResult> {
	try {
		return await graphQLClient.request<TResult>(print(document), variables);
	} catch (error) {
		if (error instanceof ClientError && error.response.status === 401) {
			redirectToLogin();
		}
		throw error;
	}
}
