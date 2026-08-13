# OMP Switch

OMP Switch is a safe structured editor for OMP configuration. It exposes supported configuration concepts without replacing OMP or normalizing configuration it does not understand.

## Language

**MVP**:
The first publicly usable release of OMP Switch; every stated safety, compatibility, platform, and acceptance requirement is release-blocking.
_Avoid_: 第一阶段原型、技术预览

**Custom Provider**:
A `models.yml.providers` entry with a non-empty model list, IDs that do not collide case-insensitively with OMP's bundled Provider/model catalog, and only configuration supported by OMP Switch. OMP Switch creates it atomically with its first Model definition and never persists an empty Custom Provider.
_Avoid_: Built-in Provider override, Provider preset

**Built-in Provider override**:
A `models.yml.providers` entry that overrides a bundled OMP Provider or bundled model through a matching Provider/model ID. OMP Switch preserves existing overrides as read-only and prevents new collisions.
_Avoid_: Custom Provider

**Target configuration**:
The global OMP agent configuration directory returned by the selected OMP executable's `omp config path` command. OMP Switch treats that command result as authoritative and does not manage project-level `.omp/config.yml`.
_Avoid_: Hard-coded configuration path, workspace configuration, project configuration

**Model definition**:
A model entry owned by one Provider, addressable through a Provider/model selector, and responsible for selecting the API protocol used for that model.
_Avoid_: Model instance, endpoint

**Supported protocol**:
One of the four model API protocols that the MVP can create, edit, and test: `openai-completions`, `openai-responses`, `anthropic-messages`, or `google-generative-ai`.
_Avoid_: Any protocol accepted by OMP

**Direct API Key**:
A text API key stored directly in a custom Provider's `apiKey` field and used literally by OMP Switch for model testing. OMP Switch does not offer environment-variable references or command-based credentials.
_Avoid_: Environment credential, command credential

**Model role**:
A named OMP workload assignment whose value selects a model and may include a Thinking Level.
_Avoid_: Agent, preset

**Simple role selector**:
A single `provider/model` selector with an optional supported Thinking Level suffix, such as `dnslin/gpt-5.6-sol:max`. Role aliases, candidate lists, arrays, and other selector structures are advanced role configuration.
_Avoid_: Role chain

**Advanced role configuration**:
Any `modelRoles` value that is not a Simple role selector. Its presence makes the MVP role page read-only so OMP Switch cannot partially overwrite role semantics it does not support.
_Avoid_: Invalid role

**Supported Thinking Level**:
One of `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, or `auto`. `ultra` is not a Thinking Level; OMP would parse an `:ultra` suffix as part of the Model ID.
_Avoid_: `ultra`, `ultrathink`

**Stable ID**:
The immutable Provider ID or Model ID of an existing object. Provider IDs must be unique globally without regard to letter case; Model IDs must be unique within their Provider without regard to letter case. Changing an ID means creating a new object and deleting the old one after references are handled.
_Avoid_: Renameable name, case-only variant

**Supported field**:
An OMP configuration field whose meaning and safe mutation rules are explicitly understood by OMP Switch.
_Avoid_: Known field

**Unrecognized configuration**:
Configuration data that OMP Switch cannot interpret safely and therefore must not silently modify or remove.
_Avoid_: Garbage field, invalid field

**Configuration transaction**:
A recoverable change that may update `models.yml` and `config.yml` together and must leave either every intended file committed or every file restored from the same transaction backup.
_Avoid_: Best-effort multi-file save

**Safe structured edit**:
A user-requested configuration change that preserves unrelated configuration and refuses to overwrite incompatible or externally changed data.
_Avoid_: Normalize, repair, clean up
