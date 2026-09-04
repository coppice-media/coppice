import { createContext } from 'svelte';
import type { LibrariesQuery, MeQuery } from '$lib/graphql/generated/graphql';
import { initialProgressState, type ProgressState } from '$lib/ingest/progress';

export type EditorUser = MeQuery['me'];
export type LibrarySummary = LibrariesQuery['libraries']['nodes'][number];

export interface EditorSession {
	selectedLibraryId: string;
	libraries: LibrarySummary[];
	user: EditorUser | null;
	progress: ProgressState;
	liveError: string | null;
}

export function createEditorSession(): EditorSession {
	let selectedLibraryId = $state('');
	let libraries = $state<LibrarySummary[]>([]);
	let user = $state<EditorUser | null>(null);
	let progress = $state<ProgressState>(initialProgressState());
	let liveError = $state<string | null>(null);

	return {
		get selectedLibraryId() {
			return selectedLibraryId;
		},
		set selectedLibraryId(value: string) {
			selectedLibraryId = value;
		},
		get libraries() {
			return libraries;
		},
		set libraries(value: LibrarySummary[]) {
			libraries = value;
		},
		get user() {
			return user;
		},
		set user(value: EditorUser | null) {
			user = value;
		},
		get progress() {
			return progress;
		},
		set progress(value: ProgressState) {
			progress = value;
		},
		get liveError() {
			return liveError;
		},
		set liveError(value: string | null) {
			liveError = value;
		}
	};
}

export const [getEditorSession, setEditorSession] = createContext<EditorSession>();
