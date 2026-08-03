/**
 * When a session's cost is worth showing.
 *
 * The console accumulates `total_cost_usd` off the CLI stream and used to show
 * it always. That number is what the tokens would have cost at API list prices
 * — and a session started by this console inherits the CLI's own credentials,
 * so it runs on the subscription and **nothing is billed per token**. A figure
 * in dollars, on a screen, next to a session, reads as a bill. It was one:
 * `$3` against a subscription that had charged nothing.
 *
 * So it is hidden while the account is inside its allowance, and appears when
 * the account itself says the allowance is running out — at which point the
 * same number stops being decorative, because that is when the work either
 * waits for the window to reset or is paid for.
 *
 * ⚠ The three statuses are the CLI's own vocabulary, read off the 2.1.220
 * binary rather than invented: `allowed`, `allowed_warning`, `rejected`.
 * Anything unrecognised — a status a later CLI adds — counts as not mattering,
 * because the failure that matters here is showing a false bill, not omitting a
 * true one.
 */
export function costMatters(limit: string | undefined): boolean {
  return limit === 'allowed_warning' || limit === 'rejected';
}
