import { browser } from '$app/environment';
import { resolve } from '$app/paths';
import { ClientError, GraphQLClient } from 'graphql-request';
import { print } from 'graphql';
import type { TypedDocumentNode } from '@graphql-typed-document-node/core';
import { StageIngestUploadsDocument } from './generated/graphql';
import type {
	StageIngestUploadsMutation,
	StageIngestUploadsMutationVariables
} from './generated/graphql';

const endpoint = browser ? `${window.location.origin}/api/graphql` : '/api/graphql';

export const graphQLClient = new GraphQLClient(endpoint, {
	credentials: 'include'
});

function redirectToLogin(): void {
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

export interface UploadFileInput {
	file: File;
	relativePath?: string;
}

export async function stageIngestUploads(
	input: Omit<StageIngestUploadsMutationVariables['input'], 'files'> & {
		files: UploadFileInput[];
	}
): Promise<StageIngestUploadsMutation['stageIngestUploads']> {
	const files = input.files;
	const variables = {
		input: {
			...input,
			files: files.map(({ relativePath }) => ({ file: null, relativePath }))
		}
	};
	const form = new FormData();
	form.append(
		'operations',
		JSON.stringify({ query: print(StageIngestUploadsDocument), variables })
	);
	const map: Record<string, string[]> = {};
	files.forEach((_entry, index) => {
		map[String(index)] = [`variables.input.files.${index}.file`];
	});
	form.append('map', JSON.stringify(map));
	files.forEach(({ file }, index) => form.append(String(index), file, file.name));

	const response = await fetch(endpoint, {
		method: 'POST',
		body: form,
		credentials: 'include'
	});
	let payload: {
		data?: StageIngestUploadsMutation;
		errors?: Array<{ message: string; extensions?: { code?: string } }>;
	};
	try {
		payload = await response.json();
	} catch {
		if (response.status === 401) redirectToLogin();
		throw new Error(`Upload failed with HTTP ${response.status}`);
	}
	if (response.status === 401) redirectToLogin();
	if (!response.ok || payload.errors?.length || !payload.data) {
		const message = payload.errors?.map((error) => error.message).join('; ')
			|| `Upload failed with HTTP ${response.status}`;
		throw new Error(message);
	}
	return payload.data.stageIngestUploads;
}
