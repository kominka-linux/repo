# TODO

## Packages
- Source mirror: fetch and rehost all sources to avoid upstream dependency
  - could also trim fat to help lower disk requirements
    - boringssl-0.20260327.0 is a prime example; massive test/ dirs that can be removed
    - we can also standardize on bzip2
- builds should be done in a linux namespace container and in a maximally reproducible manner

## Infrastructure
- Server health monitoring / alerting

