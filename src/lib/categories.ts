import type { ThreadSummary } from "./api";

/** Smart Inbox (maquette) : classement heuristique côté client, sans IA ni serveur.
 *  Remplaçable plus tard par un vrai classifieur sans toucher à l'interface. */
export type Category = "all" | "priority" | "newsletters" | "notifications";

export const CATEGORY_LABELS: Record<Category, string> = {
  all: "Tout",
  priority: "Prioritaires",
  newsletters: "Newsletters",
  notifications: "Notifications",
};

const NOTIFICATION = /(no-?reply|do-?not-?reply|notification|alert|notify|billing|receipt|invoice|security|github|gitlab|jira|slack|stripe|calendar|mailer-daemon)/i;
const NEWSLETTER = /(newsletter|digest|weekly|hebdo|substack|medium\.com|marketing|promo|offres?|deals?|updates?@|hello@|news@|info@|%\s?off|-\s?\d+\s?%|webinar)/i;

export function categorize(t: Pick<ThreadSummary, "fromAddr" | "fromName" | "subject">): Exclude<Category, "all"> {
  const sender = `${t.fromAddr ?? ""} ${t.fromName ?? ""}`;
  if (NOTIFICATION.test(sender)) return "notifications";
  if (NEWSLETTER.test(sender) || NEWSLETTER.test(t.subject)) return "newsletters";
  return "priority";
}
