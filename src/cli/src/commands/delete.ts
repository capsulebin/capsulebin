/**
 * `capsulebin delete <management link>`: delete a capsule with its delete
 * secret. The secret travels in the request body, never in the URL, and the
 * server answers an unknown id and a wrong secret identically.
 */

import { LinkError, parseLink, PasteApi, toBase64url } from '@capsulebin/core';
import { usage } from '../exit';
import type { Io } from '../io';
import { jsonLine } from '../terminal';
import { checkTransport } from '../transport';

export interface DeleteOptions {
    link: string;
    /** Plain HTTP to a host other than loopback was opted into. */
    allowHttp: boolean;
    json: boolean;
}

export async function remove(io: Io, options: DeleteOptions): Promise<void> {
    let link;
    try {
        link = parseLink(options.link, 'manage');
    } catch (err) {
        if (err instanceof LinkError) {
            throw usage(
                `invalid management link: ${err.message}` +
                    (options.link.includes('/p/') ? ' (this looks like a share link)' : ''),
            );
        }
        throw err;
    }
    checkTransport(link.base, options.allowHttp, "the link's server");

    await new PasteApi(link.base).delete(link.id, toBase64url(link.secret));

    if (options.json) {
        io.stdout.write(jsonLine({ id: link.id, deleted: true }));
    } else {
        io.stderr.write('capsule deleted\n');
    }
}
