<?php
/** Narrow adapter for the pinned upstream importer. Input is private JSON on stdin. */
declare(strict_types=1);

function validateBankConfiguration(array $c): void
{
    if (($c['flow'] ?? '') !== 'eb' || empty($c['accounts']) || empty($c['enable_banking_sessions'])
        || ($c['ignore_duplicate_transactions'] ?? false) !== true
        || ($c['duplicate_detection_method'] ?? '') !== 'classic') {
        throw new RuntimeException('invalid_configuration');
    }
    foreach ($c['accounts'] as $target) {
        if (!is_int($target) || $target <= 0) {
            throw new RuntimeException('invalid_mapping');
        }
    }
}

function normalizedBankConfiguration(array $c): array
{
    validateBankConfiguration($c);
    // Preserve source identities and reviewed target IDs. Never create asset accounts.
    $c['new_accounts'] = [];
    $c['access_token'] = '';
    $c['pending_transactions'] = false;
    $c['webhooks'] = false;
    // CLI fromArray() does not calculate relative bounds as the browser does.
    $c['date_range'] = 'range';
    $c['date_range_number'] = 30;
    $c['date_range_unit'] = 'd';
    $c['date_range_not_after_unit'] = '';
    $c['date_range_not_after_number'] = 0;
    $c['date_not_before'] = gmdate('Y-m-d', time() - 30 * 86400);
    $c['date_not_after'] = gmdate('Y-m-d', time() + 86400);
    return $c;
}

function bankPreflight(array $c, callable $get): void
{
    validateBankConfiguration($c);
    $sources = [];
    foreach ($c['enable_banking_sessions'] as $session) {
        $data = $get('bank', 'sessions/'.rawurlencode($session));
        if (($data['status'] ?? '') !== 'AUTHORIZED') {
            throw new RuntimeException('reauthorization_required');
        }
        foreach ($data['accounts'] ?? [] as $account) {
            $sources[] = is_string($account) ? $account : ($account['uid'] ?? '');
        }
    }
    foreach ($c['accounts'] as $source => $target) {
        if (!in_array($source, $sources, true)) {
            throw new RuntimeException('mapping_mismatch');
        }
        $details = $get('bank', 'accounts/'.rawurlencode($source).'/details');
        $ledger = $get('ledger', 'accounts/'.$target)['data']['attributes'] ?? [];
        $iban = strtoupper(str_replace(' ', '', $details['account_id']['iban'] ?? ''));
        $ledgerIban = strtoupper(str_replace(' ', '', $ledger['iban'] ?? ''));
        if ($iban === '' || $iban !== $ledgerIban || ($ledger['type'] ?? '') !== 'asset'
            || ($ledger['active'] ?? false) !== true) {
            throw new RuntimeException('mapping_mismatch');
        }
        verifyBankPages($source, $c, $get);
    }
}

function verifyBankPages(string $source, array $c, callable $get): void
{
    $base = 'accounts/'.rawurlencode($source).'/transactions?'.http_build_query([
        'date_from' => $c['date_not_before'], 'date_to' => $c['date_not_after'],
    ]);
    $key = ''; $seen = [];
    for ($page = 0; $page < 50; $page++) {
        $data = $get('bank', $base.($key === '' ? '' : '&continuation_key='.rawurlencode($key)));
        if (!isset($data['transactions']) || !is_array($data['transactions'])) {
            throw new RuntimeException('upstream_unavailable');
        }
        $key = (string)($data['continuation_key'] ?? '');
        if ($key === '') { return; }
        if (isset($seen[$key])) { throw new RuntimeException('provider_pagination_error'); }
        $seen[$key] = true;
    }
    throw new RuntimeException('provider_pagination_error');
}

function bankWorkerMain(): void
{
    $temporary = null;
    $result = ['status' => 'failed'];
    $phase = 'preflight';
    ob_start();
    try {
        if (function_exists('pcntl_async_signals')) {
            pcntl_async_signals(true);
            pcntl_signal(SIGTERM, static function (): void { throw new RuntimeException('worker_stopped'); });
        }
        require '/var/www/html/vendor/autoload.php';
        $app = require '/var/www/html/bootstrap/app.php';
        $kernel = $app->make(Illuminate\Contracts\Console\Kernel::class);
        $kernel->bootstrap();
        if (config('importer.version') !== '2.3.4') { throw new RuntimeException('unsupported_importer_version'); }
        config(['eb.private_key' => '/run/secrets/enable-banking-private-key.pem', 'logging.default' => 'null', 'importer.import_dir_allowlist' => ['/tmp'], 'importer.ignore_duplicate_errors' => true]);
        $input = json_decode(file_get_contents('php://stdin'), true, 512, JSON_THROW_ON_ERROR);
        $c = normalizedBankConfiguration($input['configuration']);
        $jwt = App\Services\EnableBanking\JWTManager::generateToken();
        $client = new GuzzleHttp\Client(['timeout' => 30, 'connect_timeout' => 10, 'http_errors' => false]);
        $get = function (string $system, string $path) use ($client, $jwt): array {
            $base = $system === 'bank' ? 'https://api.enablebanking.com' : rtrim(getenv('FIREFLY_III_URL'), '/').'/api/v1';
            $token = $system === 'bank' ? $jwt : getenv('FIREFLY_III_ACCESS_TOKEN');
            $r = $client->get($base.'/'.$path, ['headers' => ['Authorization' => 'Bearer '.$token, 'Accept' => 'application/json']]);
            if ($r->getStatusCode() !== 200) {
                throw new RuntimeException($r->getStatusCode() === 429 ? 'rate_limited' : 'upstream_unavailable');
            }
            return json_decode((string)$r->getBody(), true, 512, JSON_THROW_ON_ERROR);
        };
        bankPreflight($c, $get);
        if (($input['check_only'] ?? true) === true) {
            $result = ['status' => 'verified'];
        } else {
            umask(0077);
            $temporary = tempnam('/tmp', 'pfa-bank-');
            file_put_contents($temporary, json_encode($c, JSON_THROW_ON_ERROR));
            $phase = 'import';
            $exit = $kernel->call('importer:import', ['config' => $temporary], new Symfony\Component\Console\Output\NullOutput());
            // Exit 73 can conceal conversion failure in importer 2.3.4. Never mark it successful.
            $result = ['status' => $exit === 0 ? 'success' : 'import_failed', 'exit_code' => $exit];
            $phase = 'summary';
            $command = $kernel->all()['importer:import'];
            $property = new ReflectionProperty($command, 'importJob');
            if ($property->isInitialized($command)) {
                $job = $property->getValue($command);
                $result['converted'] = count($job->getConvertedTransactions());
                $result['conversion_errors'] = count($job->conversionStatus->errors);
                $result['conversion_warnings'] = count($job->conversionStatus->warnings);
                $result['submission_errors'] = count($job->submissionStatus->errors);
                $result['submission_messages'] = count($job->submissionStatus->messages);
                $result['status'] = completedBankImport($exit, $job->conversionStatus->toArray(), $job->submissionStatus->toArray()) ? 'success' : 'import_failed';
            }
        }
    } catch (Throwable $e) {
        $safe = ['reauthorization_required', 'mapping_mismatch', 'invalid_configuration', 'invalid_mapping', 'rate_limited', 'upstream_unavailable', 'provider_pagination_error', 'unsupported_importer_version'];
        $result = ['status' => in_array($e->getMessage(), $safe, true) ? $e->getMessage() : 'failed'];
        $result['phase'] = $phase;
        $result['exception_type'] = (new ReflectionClass($e))->getShortName();
    } finally {
        if (is_string($temporary)) {
            unlink($temporary);
        }
        ob_end_clean();
    }
    echo json_encode($result).PHP_EOL;
}

function completedBankImport(int $exit, array $conversion, array $submission): bool
{
    // The CLI reports 73 for clean duplicate-only runs AND some failed downloads.
    // Require completed stages and no warnings/errors/rate limits to distinguish them.
    return in_array($exit, [0, 73], true)
        && ($conversion['status'] ?? '') === 'conv_done'
        && ($submission['status'] ?? '') === 'submission_done'
        && empty($conversion['errors']) && empty($conversion['warnings'])
        && empty($conversion['rate_limits'])
        && empty($submission['errors']) && empty($submission['warnings']);
}

if (!defined('PFA_BANK_WORKER_TEST')) {
    bankWorkerMain();
}
