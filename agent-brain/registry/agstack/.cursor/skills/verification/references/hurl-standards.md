# Hurl functional test standards

Load this file when you write or review a `.verify/functional/<feature>.hurl` file.

## Rules

1. **Assert the status of every request.** Use `HTTP <code>` for an exact status, or `HTTP *` with a `[Asserts]` status check when the status is a range.
2. **Assert headers and body.** Check the content type and the keys and values the feature promises.
3. **Capture what later requests need.** Capture created identifiers in `[Captures]` and reuse them. Captures are scoped to one file.
4. **Clean up what you create.** End the scenario by removing the resources it created, unless the feature has no delete operation. Say in the feature contract when no cleanup exists.
5. **Keep requests independent of run order.** Each request uses an identifier unique to the run, so a repeat run does not collide with earlier data.

## Example

```hurl
POST {{base_url}}/api/v1/jobs
Content-Type: application/json
{ "name": "backup" }
HTTP 201
[Captures]
job_id: jsonpath "$.id"
[Asserts]
header "Content-Type" contains "application/json"

GET {{base_url}}/api/v1/jobs/{{job_id}}
HTTP 200
[Asserts]
jsonpath "$.id" == "{{job_id}}"
jsonpath "$.status" == "queued"
```

The variables (`base_url`, `job_id`) come from the environment file or from captures. Commands in the contract name the literal file.

## Negative cases

For each validation rule, add a request that breaks the rule and assert the rejection status and error code. A rule with no negative case is not tested.
