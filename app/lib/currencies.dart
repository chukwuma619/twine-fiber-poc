class FiatCurrency {
  const FiatCurrency({required this.code, required this.name});

  final String code;
  final String name;

  String get label => '$code · $name';
}

const fiatCurrencies = [
  FiatCurrency(code: 'NGN', name: 'Nigerian Naira'),
  FiatCurrency(code: 'GHS', name: 'Ghanaian Cedi'),
  FiatCurrency(code: 'KES', name: 'Kenyan Shilling'),
  FiatCurrency(code: 'UGX', name: 'Ugandan Shilling'),
  FiatCurrency(code: 'TZS', name: 'Tanzanian Shilling'),
  FiatCurrency(code: 'RWF', name: 'Rwandan Franc'),
  FiatCurrency(code: 'ZAR', name: 'South African Rand'),
  FiatCurrency(code: 'XOF', name: 'West African CFA'),
  FiatCurrency(code: 'XAF', name: 'Central African CFA'),
  FiatCurrency(code: 'EGP', name: 'Egyptian Pound'),
  FiatCurrency(code: 'MAD', name: 'Moroccan Dirham'),
  FiatCurrency(code: 'USD', name: 'US Dollar'),
  FiatCurrency(code: 'EUR', name: 'Euro'),
  FiatCurrency(code: 'GBP', name: 'British Pound'),
  FiatCurrency(code: 'CAD', name: 'Canadian Dollar'),
  FiatCurrency(code: 'AUD', name: 'Australian Dollar'),
  FiatCurrency(code: 'INR', name: 'Indian Rupee'),
  FiatCurrency(code: 'PHP', name: 'Philippine Peso'),
  FiatCurrency(code: 'IDR', name: 'Indonesian Rupiah'),
  FiatCurrency(code: 'BRL', name: 'Brazilian Real'),
  FiatCurrency(code: 'MXN', name: 'Mexican Peso'),
  FiatCurrency(code: 'CNY', name: 'Chinese Yuan'),
  FiatCurrency(code: 'JPY', name: 'Japanese Yen'),
];
