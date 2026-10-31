import datetime,hashlib,http.client,json,pathlib
cases=[]
variants=[('absent',[]),('empty',['']),('zero',['0']),('negative',['-1']),('future',['99999']),('overflow',['9223372036854775808']),('invalid',['x']),('space',[' 1']),('plus',['+1']),('leading-zero',['01']),('comma',['1,2']),('repeated',['1','2']),('invalid-then-valid',['x','1']),('valid-then-invalid',['1','x'])]
for name,path in [('params','/cosmos/bank/v1beta1/params'),('handler-error','/cosmos/bank/v1beta1/balances/bad'),('unknown','/not-a-route')]:
 for variant,heights in variants:
  c=http.client.HTTPConnection('127.0.0.1',5317,timeout=10)
  c.putrequest('GET',path)
  for value in heights:c.putheader('x-cosmos-block-height',value)
  c.endheaders();r=c.getresponse();body=r.read(1<<20);headers=r.getheaders();c.close()
  cases.append({'name':name+':'+variant,'path':path,'heights':heights,'status':r.status,'headers':headers,'body':body.decode()})
pathlib.Path('/workspace/scratch/rpc-181/rest-boundaries/rest-heights-go.json').write_text(json.dumps({'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'source_sha256':hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),'cases':cases},indent=2)+'\n')
for c in cases:
 print(c['name'],c['status'],{k:v for k,v in c['headers'] if k.lower().startswith('grpc-')},c['body'][:180])
