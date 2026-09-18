#!/usr/bin/env python3
"""Emit SYNTHETIC full-information cases to exercise replay; never a provider benchmark."""
import json
import sys

descriptor = dict(task_type='implementation', language='python', framework=None, repo_size=1,
                  candidate_files=[], estimated_scope=1, estimated_context=500, complexity='normal',
                  requires_architecture_change=False, requires_browser=False, requires_web=False,
                  tests_available=True, ambiguity=.1, long_horizon=False)

def candidate(name, tokens):
    return dict(id=name, agent=name, provider='openai', model='synthetic-model', reasoning_level=None,
                mode=None, session_strategy='fresh', context_strategy='filesystem',
                success_probability=.9, expected_tokens=tokens, expected_cost=tokens/.9,
                confidence=.8, reasons=['Synthetic fixture only'])

cases = []
for n in range(80):
    cases.append(dict(id=f'synthetic-{n}', descriptor=descriptor, arms=[
        dict(candidate=candidate('fixture-a', 1000), success=n < 20, tokens=900 if n < 20 else 1800, duration_ms=100),
        dict(candidate=candidate('fixture-b', 1100), success=True, tokens=1200, duration_ms=100),
    ]))
json.dump(dict(schema_version=1, provenance='Synthetic drift fixture; not measured provider performance', synthetic=True, cases=cases), sys.stdout, indent=2)
print()
