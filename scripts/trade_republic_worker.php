<?php
/** Private Firefly transport for the native Trade Republic adapter. */
try {
    require '/var/www/html/vendor/autoload.php';
    $input=json_decode(file_get_contents('php://stdin'),true,512,JSON_THROW_ON_ERROR);
    $client=new GuzzleHttp\Client(['timeout'=>30,'http_errors'=>false]);
    $request=function(string $method,string $path,?array $body=null)use($client):array{
        $options=['headers'=>['Authorization'=>'Bearer '.getenv('FIREFLY_III_ACCESS_TOKEN'),'Accept'=>'application/json']];
        if($body!==null)$options['json']=$body;
        $r=$client->request($method,rtrim(getenv('FIREFLY_III_URL'),'/').'/api/v1/'.$path,$options);
        if(!in_array($r->getStatusCode(),[200,201],true))throw new RuntimeException('ledger_request_failed');
        return json_decode((string)$r->getBody(),true,512,JSON_THROW_ON_ERROR);
    };
    $target=$input['target'];
    if(!is_int($target)||$target<1)throw new RuntimeException('invalid_mapping');
    $account=$request('GET','accounts/'.$target)['data'];
    if($account['attributes']['type']!=='asset'||$account['attributes']['active']!==true)throw new RuntimeException('mapping_mismatch');
    $records=[];
    for($page=1;$page<=100;$page++){
        $r=$request('GET','accounts/'.$target.'/transactions?limit=100&page='.$page);
        $records=array_merge($records,$r['data']);
        if($page>=$r['meta']['pagination']['total_pages'])break;
        if($page===100)throw new RuntimeException('ledger_pagination_error');
    }
    if(($input['operation']??'')==='snapshot'){
        $assets=$request('GET','accounts?type=asset&limit=100');
        if($assets['meta']['pagination']['total_pages']>1)throw new RuntimeException('ledger_pagination_error');
        echo json_encode(['account'=>$account,'transactions'=>$records,'assets'=>$assets['data'],'ledger_complete'=>true]);
    }elseif(($input['operation']??'')==='post'){
        $hash=hash('sha256',strtoupper(preg_replace('/\s+/','',$account['attributes']['iban'])));
        if(!hash_equals($hash,$input['iban_sha256']??''))throw new RuntimeException('mapping_mismatch');
        $t=$input['transaction'];$id=$t['external_id']??'';
        if(!preg_match('/^pfa-pytr:[a-f0-9]{64}$/',$id))throw new RuntimeException('invalid_reference');
        if(!in_array($t['type'],['withdrawal','deposit','transfer'],true)||($t['source_id']??null)!==$target&&($t['destination_id']??null)!==$target)throw new RuntimeException('invalid_mapping');
        foreach($records as $group)foreach($group['attributes']['transactions']as$existing){
            if(($existing['external_id']??'')===$id){
                if(bccomp($existing['amount'],$t['amount'],8)!==0||$existing['currency_code']!==$t['currency_code']||$existing['type']!==$t['type'])throw new RuntimeException('existing_event_changed');
                echo json_encode(['status'=>'existing']);exit;
            }
        }
        $r=$request('POST','transactions',['error_if_duplicate_hash'=>true,'apply_rules'=>false,'fire_webhooks'=>true,'transactions'=>[$t]]);
        $created=$r['data']['attributes']['transactions']??[];
        if(count($created)!==1||($created[0]['external_id']??'')!==$id)throw new RuntimeException('unconfirmed_post');
        echo json_encode(['status'=>'created']);
    }else throw new RuntimeException('invalid_operation');
}catch(Throwable $e){echo json_encode(['status'=>'ledger_operation_failed']);exit(1);}
