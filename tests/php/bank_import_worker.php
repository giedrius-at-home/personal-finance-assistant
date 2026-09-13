<?php
// Concatenate after the adapter with PFA_BANK_WORKER_TEST defined. All data synthetic.
function expect(bool $condition): void { if (!$condition) { throw new RuntimeException('test_failed'); } }
$fixture = ['flow'=>'eb', 'accounts'=>['synthetic-source'=>42], 'enable_banking_sessions'=>['synthetic-consent'], 'ignore_duplicate_transactions'=>true, 'duplicate_detection_method'=>'classic', 'new_accounts'=>['synthetic-source'=>['create'=>'1']], 'access_token'=>'synthetic-secret'];
$normalized = normalizedBankConfiguration($fixture);
expect($normalized['accounts'] === $fixture['accounts']);
expect($normalized['new_accounts'] === [] && $normalized['access_token'] === '');
expect($normalized['pending_transactions'] === false && $normalized['date_range_number'] === 30);
expect($normalized['date_range'] === 'range' && $normalized['date_not_before'] !== '' && $normalized['date_not_after'] !== '');
$response = function (string $system, string $path): array {
    if (str_starts_with($path, 'sessions/')) { return ['status'=>'AUTHORIZED','accounts'=>['synthetic-source']]; }
    if (str_contains($path, '/transactions?')) { return ['transactions'=>[], 'continuation_key'=>null]; }
    if ($system === 'bank') { return ['account_id'=>['iban'=>'SYNTHETIC-IBAN']]; }
    return ['data'=>['attributes'=>['iban'=>'SYNTHETIC-IBAN','type'=>'asset','active'=>true]]];
};
bankPreflight($normalized, $response);
foreach (['expired','wrong_iban','missing_source','new_target','duplicates_off'] as $case) {
    $c = $normalized;
    if ($case === 'new_target') { $c['accounts']['synthetic-source'] = 0; }
    if ($case === 'duplicates_off') { $c['ignore_duplicate_transactions'] = false; }
    $rejected = false;
    try {
        bankPreflight($c, function ($system, $path) use ($response, $case) {
            $r = $response($system, $path);
            if ($case === 'expired' && isset($r['status'])) { $r['status'] = 'EXPIRED'; }
            if ($case === 'missing_source' && isset($r['accounts'])) { $r['accounts'] = []; }
            if ($case === 'wrong_iban' && $system === 'ledger') { $r['data']['attributes']['iban'] = 'OTHER-SYNTHETIC'; }
            return $r;
        });
    } catch (RuntimeException $e) { $rejected = true; }
    expect($rejected);
}
echo "Bank worker synthetic tests passed\n";
$conversion = ['status'=>'conv_done'];
$submission = ['status'=>'submission_done'];
expect(completedBankImport(73, $conversion, $submission));
expect(!completedBankImport(73, ['status'=>'conv_errored'], $submission));
expect(!completedBankImport(0, $conversion + ['warnings'=>['synthetic network failure']], $submission));
expect(!completedBankImport(0, $conversion, $submission + ['errors'=>['synthetic invalid field']]));
expect(!completedBankImport(1, $conversion, $submission));
$calls = 0;
try {
    verifyBankPages('synthetic-source', $normalized, function () use (&$calls) { $calls++; return ['transactions'=>[], 'continuation_key'=>'synthetic-loop']; });
    throw new LogicException('Expected repeated pagination to fail');
} catch (RuntimeException $e) { expect($e->getMessage() === 'provider_pagination_error' && $calls === 2); }
$calls = 0;
verifyBankPages('synthetic-source', $normalized, function () use (&$calls) { $calls++; return ['transactions'=>[], 'continuation_key'=>$calls === 1 ? 'synthetic-next' : null]; });
expect($calls === 2);
