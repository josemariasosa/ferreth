# Agent Roles

Defined agent roles and their responsibilities. Each role specifies:

- Capabilities and limitations
- Authorized file paths (subset of repository)
- Decision boundaries
- Communication protocols with other agents

## Role Structure

Each role definition should include:

```markdown
## <Role Name>

### Purpose
<What this agent is responsible for>

### Capabilities
- List of operations allowed
- Tools that can be used

### Restrictions
- Files/areas the agent cannot modify
- Decision types that require escalation

### Communication
<How this role interacts with others>
```
