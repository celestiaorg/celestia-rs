#!/usr/bin/env python3
"""Verify bounded tracker completion without asserting universal readiness."""
import hashlib
import importlib.util
import json
from pathlib import Path

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
def verify():
    record=json.loads((HERE/'readiness.json').read_text())
    if record['tracking_issue']!=181 or record['bounded_baseline_complete'] is not True or record['universal_equivalence_established'] is not False:raise ValueError('readiness scope changed')
    for name,sha in record['bindings'].items():
        path=(ROOT/name).resolve()
        if not path.is_relative_to(ROOT) or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest()!=sha:raise ValueError('changed readiness binding: '+name)
    spec=importlib.util.spec_from_file_location('readiness_audit',HERE/'scope_audit.py');audit=importlib.util.module_from_spec(spec);spec.loader.exec_module(audit)
    matrix=audit.validate(json.loads(audit.AUDIT.read_text()))
    if matrix['provider_ready'] is not False or {row['issue'] for row in matrix['behaviors']}!=set(range(182,193)):raise ValueError('incomplete or promoted behavior matrix')
    if {row['id'] for row in matrix['packages']}!=set(record['packages']):raise ValueError('completion package set differs')
    for issue,required in {188:'module-closure',189:'persistent-provider-closure',192:'provider-soak-closure'}.items():
        row=next(row for row in matrix['behaviors'] if row['issue']==issue)
        if required not in row['packages']:raise ValueError('missing closure package')
    for result in record['validation']:
        log=(ROOT/result['log']).read_text()
        if result['exit_code']!=0 or result['success_marker'] not in log:raise ValueError('validation did not pass')
    print('Bounded #181 baseline complete: 19 scoped packages, current-release matched-state evidence and final all-API soak; universal provider equivalence remains unestablished')
if __name__=='__main__':verify()
