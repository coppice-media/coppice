import { browser } from '$app/environment';
import { createQuery } from '@tanstack/svelte-query';

/**
 * The public slice of the server's OIDC settings, `GET /api/v2/auth/oidc/config`
 * (`apps/server/src/routers/api/v2/oidc.rs`). It says whether single sign-on
 * is configured, never how provider groups map to permissions: the mapping
 * and `STUMP_OIDC_SYNC_PERMISSIONS` stay on the server.
 */
export interface OidcConfig {
	enabled: boolean;
	allowRegistration: boolean;
	disableLocalAuth: boolean;
}

const OIDC_UNAVAILABLE: OidcConfig = {
	enabled: false,
	allowRegistration: false,
	disableLocalAuth: false
};

/**
 * The OIDC settings, fetched once per page load: they only change with a
 * server restart. A non-OK answer reads as "OIDC is off"; a network failure
 * leaves `data` undefined for the caller to treat as unknown.
 */
export function createOidcConfigQuery() {
	return createQuery(() => ({
		queryKey: ['oidcConfig'],
		queryFn: async (): Promise<OidcConfig> => {
			const response = await fetch('/api/v2/auth/oidc/config', { credentials: 'include' });
			if (!response.ok) return OIDC_UNAVAILABLE;
			return (await response.json()) as OidcConfig;
		},
		enabled: browser,
		staleTime: Number.POSITIVE_INFINITY
	}));
}
