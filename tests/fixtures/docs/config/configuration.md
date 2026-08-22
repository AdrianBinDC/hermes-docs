# Configuration

Hermes Agent reads its configuration from `~/.hermes/config.yaml`.

## Configuration Files

The main configuration file lives at `~/.hermes/config.yaml`. It controls
model selection, API keys, token limits, and behavioral flags. You can keep
multiple configuration profiles and switch between them.

## Environment Variables

Most configuration keys can be overridden by environment variables, such as
`HERMES_MODEL`, `HERMES_API_KEY`, and `HERMES_HOME`. Environment variables take
precedence over values from the configuration file.

## Profiles

Profiles let you keep isolated configuration sets. Activate a profile with the
`hermes profile` command to load its configuration file.
