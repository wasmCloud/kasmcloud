package v1alpha1

import metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

type Claims struct {
	Issuer    string       `json:"issuer"`
	Subject   string       `json:"subject"`
	IssuedAt  metav1.Time  `json:"issuedAt"`
	NotBefore *metav1.Time `json:"notBefore,omitempty"`
	Expires   *metav1.Time `json:"expires,omitempty"`
}
