import { ClientError } from 'graphql-request'

/**
 * The one sentence a human should read for a failed operation.
 *
 * `request` already unwraps the first GraphQL message for operations it
 * drives, but query/mutation errors reach components from several places —
 * a raw `graphQLClient` call, an upload helper, a `fetch` rejection — so the
 * unwrap lives here too: graphql-request's own `ClientError.message` dumps
 * the whole request *and* response, which is not what a toast should show.
 */
export function errorMessage(err: unknown): string {
	if (err instanceof ClientError) {
		const message = err.response.errors?.[0]?.message
		if (message) return message
		// A transport failure (gateway, proxy, panic) carries no GraphQL
		// error. Falling through to `Error.message` here would paste the
		// whole request and response into a toast, so name the status.
		const status = err.response.status
		return status ? `The server answered ${status}.` : 'The server did not answer.'
	}
	if (err instanceof Error && err.message) return err.message
	return String(err)
}
