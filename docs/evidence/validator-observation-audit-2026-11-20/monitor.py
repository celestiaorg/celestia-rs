from datetime import datetime, timezone
from pathlib import Path
import gzip,json,urllib.request,urllib.parse
root=Path(__file__).parent
end=datetime.fromisoformat('2026-11-15T05:25:00+00:00').timestamp()
metrics=['up{job="celestia-rs"}','celestia_rs_rpc_up','celestia_rs_process_up','celestia_rs_catching_up','celestia_rs_latest_block_time_seconds','celestia_rs_collector_timestamp_seconds','celestia_rs_validator_jailed','celestia_rs_validator_tombstoned','celestia_rs_sign_refusals_total','celestia_rs_apphash_match','ALERTS{alertstate="firing"}']
summary={}
for metric in metrics:
 query=metric+'[72h]'
 url='http://127.0.0.1:9090/api/v1/query?'+urllib.parse.urlencode({'query':query,'time':end})
 with urllib.request.urlopen(url,timeout=40) as response: payload=response.read()
 data=json.loads(payload)
 assert data['status']=='success' and data['data']['resultType']=='matrix'
 name=metric.split('{')[0]
 with gzip.open(root/(name+'.json.gz'),'wb') as out: out.write(payload)
 results=[]
 for series in data['data']['result']:
  values=series['values']; times=[float(t) for t,v in values]; nums=[float(v) for t,v in values]
  gaps=[{'start':a,'end':b,'seconds':b-a} for a,b in zip([end-259200,*times],[*times,end]) if b-a>20]
  entry={'metric':series['metric'],'count':len(values),'first':values[0] if values else None,'last':values[-1] if values else None,'min':min(nums) if nums else None,'max':max(nums) if nums else None,'gaps_over_20s':gaps}
  if name in ('celestia_rs_latest_block_time_seconds','celestia_rs_collector_timestamp_seconds'):
   entry['max_timestamp_age_seconds']=max(t-float(v) for t,v in values)
  results.append(entry)
 summary[query]=results
 print(name,[(x['metric'].get('network'),x['count'],len(x['gaps_over_20s'])) for x in results],flush=True)
(root/'monitoring-summary.json').write_text(json.dumps({'scope':'Retrospective raw Prometheus matrices; missing samples stay unknown; no alert-delivery certification.','start_utc':'2026-11-12T05:25:00Z','end_utc':'2026-11-15T05:25:00Z','queries':summary},indent=2)+'\n')
