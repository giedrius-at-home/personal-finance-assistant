# Contributing

## Workflow

1. Create a short-lived branch from `main`.
2. Add or update an ADR when a change alters an architectural constraint.
3. Add tests before changing financial behaviour.
4. Run `make check` locally.
5. Open a pull request using the repository template.
6. Squash merge only after required checks pass.

Direct pushes, force pushes, and deletion of `main` are prohibited.

## Financial-data rule

Never commit real financial data, credentials, exports, screenshots, raw model
prompts containing personal records, or production database copies. Tests must
use small synthetic fixtures with obviously fictitious identities and values.

## Commit style

Use imperative, scoped messages where practical, for example:

```text
feat(goals): add deadline forecast
fix(alerts): preserve critical severity
docs(adr): record currency precision policy
```

## Definition of done

- Behaviour and failure modes are tested.
- Financial calculations use `Decimal`, never binary floating point.
- Data contracts reject unknown fields.
- Documentation and `CHANGELOG.md` reflect meaningful changes.
- The PR states verification commands and remaining risks.
