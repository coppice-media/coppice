import { print } from 'graphql';
import {
	StageIngestUploadsDocument,
	type StageIngestUploadsMutation,
	type StageIngestUploadsMutationVariables
} from '$lib/graphql/generated/graphql';
import { graphQLEndpoint, redirectToLogin } from '@stump/ui/graphql/client';

export interface UploadFileInput {
	file: File;
	relativePath?: string;
}

/**
 * Stage one or more files through the multipart GraphQL upload path. Files
 * travel outside the JSON document: the `operations` and `map` parts carry
 * null placeholders that are bound to the file parts by index.
 */
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

	const response = await fetch(graphQLEndpoint, {
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
