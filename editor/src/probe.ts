import { request } from '@stump/ui/graphql/client';
import { MeDocument } from '@stump/ui/graphql/generated';

// Type probe: if TypedDocumentNode inference flows through the shared
// package, `whoami` compiles as string; otherwise the check output shows the
// degraded type.
const result = request(MeDocument, {});
const whoami: string = result.me.username;
export { whoami };
