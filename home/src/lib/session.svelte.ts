import { createContext } from 'svelte';
import type { MeQuery } from '@stump/ui/graphql/generated/graphql';

export type HomeUser = MeQuery['me'];

export interface HomeSession {
	user: HomeUser | null;
}

export function createHomeSession(): HomeSession {
	let user = $state<HomeUser | null>(null);

	return {
		get user() {
			return user;
		},
		set user(value: HomeUser | null) {
			user = value;
		}
	};
}

export const [getHomeSession, setHomeSession] = createContext<HomeSession>();
