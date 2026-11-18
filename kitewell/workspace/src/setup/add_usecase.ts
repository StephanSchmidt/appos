// Usecase: add someone to the workspace and get their invite link.

import type { Result } from "../../bindings/abi.ts";
import type { Member } from "../../bindings/flow.ts";
import * as setup from "../../bindings/setup.ts";

/** The member with their invite link, or the booking functions' reason in
 * words (all seats taken, an email that isn't one). Adding someone twice
 * answers with the same member and link. */
export function addUc(booking: string, name: string, email: string): Result<Member, string> {
  return addU(booking, name, email, setup.addMember);
}

export function addU(
  booking: string,
  name: string,
  email: string,
  addMember: (booking: string, name: string, email: string) => Result<Member, string>,
): Result<Member, string> {
  return addMember(booking, name.trim(), email.trim());
}
